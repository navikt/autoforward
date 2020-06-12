use std::error::Error;
use std::fmt;
use std::io;
use std::process::Stdio;
use std::str::FromStr;
use std::task::Poll;
use std::time::{Duration, SystemTime};

use hyper::{Client, Uri};
use hyper::client::HttpConnector;
use nix::unistd::Pid;
use pin_utils::pin_mut;
use regex::Regex;
use tokio::{io::{AsyncBufReadExt, BufReader}};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use futures_util::stream::FuturesOrdered;

use super::kubernetes::{ApplicationResource, KubernetesResponse};
use futures_util::{FutureExt, StreamExt};

#[derive(Debug)]
pub struct ForwardError {
    message: &'static str,
    original: io::Error,
}

impl fmt::Display for ForwardError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for ForwardError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.original)
    }
}

trait ToForwardError<A> {
    fn context(self, context: &'static str) -> Result<A, ForwardError>;
}

impl<A> ToForwardError<A> for Result<A, io::Error> {
    fn context(self, context: &'static str) -> Result<A, ForwardError> {
        match self {
            Ok(v) => Ok(v),
            Err(e) => Err(ForwardError {
                message: context,
                original: e,
            }),
        }
    }
}

#[derive(PartialEq, Eq)]
struct ApplicationDescriptor {
    application_name: String,
    ingresses: Vec<String>,
    liveness: Option<String>,
    context: String,
    namespace: String,
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
            .args(&["port-forward",
                "--context", application.context.as_str(),
                "--namespace", application.namespace.as_str(),
                format!("svc/{}", application.application_name.as_str()).as_str(), ":80"])
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

        PortforwardDescriptor::kill(self.port_forward_command).await;
        self.stdout.await.unwrap();
    }

    #[cfg(unix)]
    async fn kill(mut process: Child) {
        let process_id = process.id();
        let output = process.wait_with_output();
        pin_mut!(output);
        nix::sys::signal::kill(Pid::from_raw(process_id as _), nix::sys::signal::SIGINT);
        if let Err(_) = timeout(Duration::from_secs(3), &mut output).await {
            println!("Failed to sigint kubectl, killing");
            nix::sys::signal::kill(Pid::from_raw(process_id as _), nix::sys::signal::SIGKILL);

            output.await.unwrap();
        }
        println!("Closed port-forward.");
    }

    #[cfg(not(unix))]
    async fn kill(mut process: Child) {
        process.kill().unwrap();
        process.wait_with_output().await.unwrap();
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

impl ApplicationDescriptor {
    fn create(resource: ApplicationResource, context: String, namespace: String) -> Self {
        ApplicationDescriptor {
            application_name: resource.metadata.name,
            ingresses: resource.spec.ingresses.unwrap().clone(),
            liveness: resource.spec.liveness.map(|v| v.path),
            context,
            namespace,
        }
    }
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

    pub async fn new(contexts: Vec<String>, namespaces: Vec<String>) -> Result<State, ForwardError> {
        let descriptors = contexts.into_iter()
            .flat_map(|context| (&namespaces).into_iter().map(move |namespace| (context.clone(), namespace.clone())))
            .map(|(context, namespace)| Self::fetch_descriptors(context.clone(), namespace.clone()))
            .collect::<FuturesOrdered<_>>()
            .collect::<Vec<_>>().await
            .into_iter()
            .flatten()
            .flatten()
            .collect::<Vec<_>>();
        Ok(State {
            next_update: State::next_update(),
            hosts: descriptors,
            port_forwards: vec![],
        })
    }

    async fn fetch_descriptors(context: String, namespace: String) -> Result<Vec<ApplicationDescriptor>, ForwardError> {
        let cmd = Command::new("kubectl")
            .args(&["--context", context.as_str(), "--namespace", namespace.as_str(), "get", "application", "-o", "json"])
            .output()
            .await
            .context("Failed to execute kubectl get application")?;
        if !cmd.status.success() {
            let input = String::from_utf8(cmd.stderr).unwrap();
            return Err(ForwardError {
                message: "Failed to execute kubectl get application, got invalid exit code. Is navtunnel running?",
                original: io::Error::new(io::ErrorKind::Other, input),
            });
        }
        let resource = serde_json::from_slice::<KubernetesResponse>(&cmd.stdout)
            .unwrap();
        Ok(resource.items
            .into_iter()
            .filter(|application| application.spec.ingresses.is_some())
            .map(|application| ApplicationDescriptor::create(application, context.clone(), namespace.clone()))
            .collect())
    }

    pub fn hostnames(&self) -> Vec<String> {
        let regex = Regex::new(r"https?://(.[^/]+)(:?/.*)?").unwrap();
        let mut hosts: Vec<String> = (&self.hosts)
            .into_iter()
            .flat_map(|v| (&v.ingresses))
            .map(|ingress| {
                let captures = regex.captures(ingress.as_str()).unwrap();
                captures[1].to_owned()
            })
            .collect();
        hosts.sort();
        hosts.dedup();
        hosts
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

    pub async fn fetch_address(&mut self, host: &String, path: &str) -> Result<Option<Portforward>, ForwardError> {
        let info = (&self.hosts).into_iter()
            .filter_map(|desc| (desc.best_ingress(host, path).map(|v| (v, desc))))
            .max_by(|(a, _), (b, _)| a.len().cmp(&b.len()));

        let (ingress, app) = if let Some(info) = info {
            info
        } else {
            return Ok(None);
        };
        let mut desc = (&mut self.port_forwards).into_iter()
            .find(|v| v.contains_ingress(&ingress));
        if let Some(desc) = &mut desc {
            desc.update_ttl();
            Ok(Some((&desc.portforward).clone()))
        } else {
            let portforward_desc: PortforwardDescriptor = PortforwardDescriptor::from_app(&app)
                .await
                .context("Could not open port-forward. Are you still connected to navtunnel?")?;
            let portforward = portforward_desc.portforward.clone();
            self.port_forwards.push(portforward_desc);
            Ok(Some(portforward))
        }
    }
}
