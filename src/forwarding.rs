use std::io;
use std::process::Stdio;
use std::str::FromStr;
use std::time::{Duration, SystemTime};

use hyper::{Client, Uri};
use hyper::client::HttpConnector;
use regex::Regex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

use super::kubernetes::{ApplicationResource, KubernetesResponse};

#[derive(PartialEq, Eq)]
struct ApplicationDescriptor {
    application_name: String,
    ingresses: Vec<String>,
    liveness: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Portforward {
    pub host: String,
    pub port: usize,
}

struct PortforwardDescriptor {
    hosts: Vec<String>,
    ttl: SystemTime,
    port_forward_command: Child,
    client: Client<HttpConnector>,
    liveness: Option<String>,
    stdout: JoinHandle<()>,
    portforward: Portforward,
}

impl PortforwardDescriptor {
    fn create_ttl() -> SystemTime {
        SystemTime::now() + Duration::from_secs(60)
    }

    async fn from_app(application: &ApplicationDescriptor) -> Result<PortforwardDescriptor, io::Error> {
        let regex = Regex::new(r"Forwarding from (.+):(\d{2,5}) -> \d{2,5}").unwrap();

        let mut cmd = Command::new("kubectl")
            .args(&["port-forward", "--context", "dev-fss", format!("svc/{}", application.application_name.as_str()).as_str(), ":80"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        let mut lines = BufReader::new(cmd.stdout.take().unwrap()).lines();
        let line = lines.next_line().await?.unwrap();
        let captures = regex.captures(line.as_str()).unwrap();
        let host = captures[1].to_owned();
        let port: usize = captures[2].parse().unwrap();

        println!("Opened a connection for {}:{} from {}", &host, &port, &line);

        Ok(PortforwardDescriptor {
            hosts: application.ingresses.clone(),
            ttl: PortforwardDescriptor::create_ttl(),
            port_forward_command: cmd,
            client: Client::new(),
            liveness: (&application).liveness.to_owned(),
            stdout: tokio::spawn(async move {
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.starts_with("Handling connection") {
                        println!("{}", line);
                    }
                }
            }),
            portforward: Portforward {
                host,
                port,
            },
        })
    }

    async fn tick(&mut self) -> bool {
        if !self.check_selftest().await {
            println!("Failed selftest, marking connection for {:?} as dead", &self.hosts);
            return false;
        }
        return self.ttl > SystemTime::now();
    }

    async fn close(mut self) {
        println!("Closing port-forward for {:?}", self.hosts);

        self.port_forward_command.kill().unwrap();
        self.port_forward_command.wait_with_output().await.unwrap();
        self.stdout.await.unwrap();
    }

    async fn check_selftest(&self) -> bool {
        if let Some(liveness) = &self.liveness {
            let path = if liveness.starts_with("/") {
                &liveness[1..]
            } else {
                liveness.as_str()
            };
            let uri = Uri::from_str(format!("http://{}:{}/{}", self.portforward.host, self.portforward.port, path).as_str());
            println!("Running self-test towards {:?}", &uri);
            let response = self.client.get(uri.unwrap()).await;
            return match response {
                Ok(response) => response.status().is_success(),
                _ => false,
            };
        }
        false
    }

    fn contains_ingress(&self, ingress: &String) -> bool {
        self.hosts.contains(ingress)
    }

    fn update_ttl(&mut self) {
        self.ttl = Self::create_ttl();
    }
}

pub struct State {
    next_update: SystemTime,
    hosts: Vec<ApplicationDescriptor>,
    port_forwards: Vec<PortforwardDescriptor>,
}

impl From<ApplicationResource> for ApplicationDescriptor {
    fn from(resource: ApplicationResource) -> ApplicationDescriptor {
        ApplicationDescriptor {
            application_name: resource.metadata.name,
            ingresses: resource.spec.ingresses.unwrap().clone(),
            liveness: resource.spec.liveness.map(|v| v.path),
        }
    }
}

impl ApplicationDescriptor {
    fn best_ingress(&self, host: &str, path: &str) -> Option<String> {
        (&self.ingresses).into_iter()
            .map(|pf| (Uri::from_str(pf.as_str()), pf))
            .filter(|(uri, _)| uri.is_ok())
            .map(|(uri, ingress)| (uri.unwrap(), ingress))
            .filter(|(uri, _)| uri.host() == Some(host))
            .filter(|(uri, _)| {
                println!("matching {} with {}, outcome {}", uri.path(), path, uri.path().len());
                path.starts_with(uri.path())
            })
            .map(|(_, ingress)| ingress.to_owned())
            .max_by(|a, b| a.len().cmp(&b.len()))
    }
}

impl State {
    fn next_update() -> SystemTime {
        SystemTime::now() + Duration::from_secs(120)
    }

    pub async fn new() -> State {
        let cmd = Command::new("kubectl")
            .args(&["--context", "dev-fss", "get", "application", "-o", "json"])
            .output()
            .await
            .unwrap();
        let resource = serde_json::from_slice::<KubernetesResponse>(&cmd.stdout)
            .unwrap();
        let descriptors: Vec<ApplicationDescriptor> = resource.items
            .into_iter()
            .filter(|application| application.spec.ingresses.is_some())
            .map(|application| application.into())
            .collect();
        State {
            next_update: State::next_update(),
            hosts: descriptors,
            port_forwards: vec![],
        }
    }

    pub fn hostnames(&self) -> Vec<String> {
        let regex = Regex::new(r"https?://(.[^/]+)(:?/.*)?").unwrap();
        (&self.hosts)
            .into_iter()
            .flat_map(|v| (&v.ingresses))
            .map(|ingress| {
                let captures = regex.captures(ingress.as_str()).unwrap();
                captures[1].to_owned()
            })
            .collect()
    }

    pub async fn tick(&mut self) {
        if self.next_update < SystemTime::now() {
            self.next_update = State::next_update();
        }
        let mut new_portforwards = Vec::with_capacity(self.port_forwards.len());
        while !self.port_forwards.is_empty() {
            let mut pf = self.port_forwards.remove(self.port_forwards.len() - 1);
            if pf.tick().await {
                new_portforwards.push(pf);
            } else {
                pf.close().await;
            }
        }
        self.port_forwards = new_portforwards;
    }

    pub async fn fetch_address(&mut self, host: &String, path: &str) -> Option<Portforward> {
        let info = (&self.hosts).into_iter()
            .filter_map(|desc| (desc.best_ingress(host, path).map(|v| (v, desc))))
            .max_by(|(a, _), (b, _)| a.len().cmp(&b.len()));

        let (ingress, app) = if let Some(info) = info {
            info
        } else {
            return None;
        };
        let mut desc = (&mut self.port_forwards).into_iter()
            .find(|v| v.contains_ingress(&ingress));
        if let Some(desc) = &mut desc {
            desc.update_ttl();
            Some((&desc.portforward).clone())
        } else {
            let portforward_desc: PortforwardDescriptor = PortforwardDescriptor::from_app(&app).await.unwrap();
            let portforward = portforward_desc.portforward.clone();
            self.port_forwards.push(portforward_desc);
            Some(portforward)
        }
    }
}