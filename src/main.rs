extern crate futures_util;
extern crate hyper;
#[cfg(unix)]
extern crate nix;
extern crate pin_utils;
extern crate regex;
extern crate rustls;
extern crate serde_json;
#[cfg(test)]
extern crate tempfile;
extern crate tokio;
extern crate tokio_rustls;

use std::{io, io::Write};
use std::convert::Infallible;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use hyper::{Body, Client, Request, Response, Server, StatusCode, Uri};
use hyper::service::{make_service_fn, service_fn};
use nix::unistd::Uid;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use forwarding::State;
use crate::forwarding::ForwardError;

mod kubernetes;
mod tls;
mod forwarding;
mod hosts;

#[cfg(unix)]
fn update_hosts_on_root(state: &State) {
    let uid = nix::unistd::getuid();
    if uid.is_root() {
        println!("Process started as root. Updating hosts entries");
        hosts::update_hosts_file(hosts::hosts_file(), &state.hostnames());
    } else {
        println!("Unable to update hosts entries, application needs to be run as root");
    }
}

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(unix)]
        let mut tcp = if nix::unistd::getuid().is_root() {
        TcpListener::bind(&"127.0.0.1:443").await?
    } else {
        TcpListener::bind(&"127.0.0.1:8443").await?
    };
    let state = {
        let state = State::new(vec!["dev-fss".to_owned(), "prod-fss".to_owned()], vec!["default".to_owned(), "tbd".to_owned()]).await?;
        #[cfg(unix)]
        update_hosts_on_root(&state);

        Arc::new(Mutex::new(state))
    };

    // TODO?: nix::unistd::setuid(Uid::from_raw(unimplemented!())).unwrap();

    let local_state = state.clone();

    tokio::spawn(async move {
        loop {
            local_state.lock().await.tick().await;
            tokio::time::delay_for(Duration::from_secs(10)).await;
        }
    });

    let service_fun = make_service_fn(move |_| {
        let inner = state.clone();
        async {
            Ok::<_, Infallible>(service_fn(move |req: Request<Body>| {
                handle_req(req, inner.clone())
            }))
        }
    });
    let server = Server::builder(tls::tls_acceptor(&mut tcp).await?)
        .serve(service_fun);

    server.await?;
    Ok(())
}

async fn handle_req(mut req: Request<Body>, state: Arc<Mutex<State>>) -> Result<Response<Body>, ForwardError> {
    let client = Client::new();
    let request_host = if let Some(host) = req.headers().get("Host") {
        host.to_str().map(|h| {
            if let Some(index) = h.find(':') {
                h[0..index].to_owned()
            } else {
                h.to_owned()
            }
        }).unwrap()
    } else {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from(format!("The proxy requires a Host header to work.")))
            .unwrap());
    };
    let uri = if let Some(portforward) = state.lock().await.fetch_address(&request_host, req.uri().path()).await? {
        let request_uri = req.uri();
        format!("http://{}:{}{}", portforward.host, portforward.port, request_uri.path())
    } else {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(format!("No service found for {}", request_host)))
            .unwrap());
    };
    println!("Handling request for {}, forwarding to {}", &request_host, &uri);
    *req.uri_mut() = Uri::from_str(uri.as_str()).unwrap();
    Ok::<_, _>(match client.request(req).await {
        Ok(value) => value,
        Err(e) => Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .body(Body::from(format!("{}", e))).unwrap(),
    })
}
