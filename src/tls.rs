use std::fs::File;
use std::io;

use futures_util::{
    future::TryFutureExt,
    stream::{Stream, StreamExt, TryStreamExt},
};
use rustls::internal::pemfile;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;
use std::sync::Arc;
use std::pin::Pin;
use std::task::{Poll, Context};

pub async fn tls_acceptor(tcp: &'_ mut TcpListener) -> Result<HyperAcceptor<'_>, io::Error> {
    let tls_cfg = {
        let certs = load_certs("cert.pem")?;
        let key = load_private_key("key.pem")?;

        let mut cfg = rustls::ServerConfig::new(rustls::NoClientAuth::new());

        cfg.set_single_cert(certs, key)
            .map_err(|e| error(format!("{}", e)))?;
        cfg.set_protocols(&[b"http/1.1".to_vec(), b"h2".to_vec()]);
        Arc::new(cfg)
    };

    let tls_acceptor = TlsAcceptor::from(tls_cfg);

    let incoming_tls_stream = tcp
        .incoming()
        .map_err(|e| error(format!("Incoming failed: {:?}", e)))
        .and_then(move |s| {
            tls_acceptor.accept(s).map_err(|e| {
                println!("Connection closed due to TLS error: ${:?}", e);
                error(format!("TLS Error: {:?}", e))
            })
        })
        .boxed();

    Ok(HyperAcceptor {
        acceptor: incoming_tls_stream
    })
}

fn load_certs(filename: &str) -> io::Result<Vec<rustls::Certificate>> {
    let certfile = File::open(filename)
        .map_err(|e| error(format!("failed to open {}: {}", filename, e)))?;
    let mut reader = io::BufReader::new(certfile);

    pemfile::certs(&mut reader).map_err(|_| error("failed to load certificate".into()))
}

fn load_private_key(filename: &str) -> io::Result<rustls::PrivateKey> {
    let keyfile = File::open(filename)
        .map_err(|e| error(format!("failed to open {}: {}", filename, e)))?;
    let mut reader = io::BufReader::new(keyfile);

    let keys = pemfile::pkcs8_private_keys(&mut reader)
        .map_err(|_| error("failed to load private key".into()))?;
    if keys.len() != 1 {
        return Err(error("expected a single private key".into()));
    }
    Ok(keys[0].clone())
}

pub struct HyperAcceptor<'a> {
    acceptor: Pin<Box<dyn Stream<Item=Result<TlsStream<TcpStream>, io::Error>> + 'a>>,
}

impl hyper::server::accept::Accept for HyperAcceptor<'_> {
    type Conn = TlsStream<TcpStream>;
    type Error = io::Error;

    fn poll_accept(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        match Pin::new(&mut self.acceptor).poll_next(cx) {
            Poll::Ready(Some(Err(_))) => Poll::Pending,
            record => record,
        }
    }
}

fn error(err: String) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err)
}
