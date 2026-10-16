//! TLS for the PostgreSQL connection, through rustls.
//!
//! The connection string's `sslmode` decides whether TLS is used (`disable`,
//! `prefer` — the default — or `require`). As in libpq for these modes, the
//! server certificate is not verified: the connection is encrypted, but not
//! authenticated. Point `postgres` at a trusted test database.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use cawit_resp::tls::TlsConfig;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::tls::{ChannelBinding, MakeTlsConnect, TlsConnect};

/// A [`MakeTlsConnect`] for `tokio_postgres::connect`.
#[derive(Clone, Debug)]
pub struct MakeRustls(TlsConfig);

impl MakeRustls {
    /// Encrypt without verifying the server certificate (libpq `sslmode=require` semantics).
    pub fn new() -> io::Result<MakeRustls> {
        Ok(MakeRustls(TlsConfig::no_verify()?))
    }
}

impl<S> MakeTlsConnect<S> for MakeRustls
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = RustlsStream<S>;
    type TlsConnect = RustlsConnect;
    type Error = io::Error;

    fn make_tls_connect(&mut self, domain: &str) -> io::Result<RustlsConnect> {
        Ok(RustlsConnect {
            config: self.0.clone(),
            domain: domain.to_string(),
        })
    }
}

/// One TLS handshake for one connection.
pub struct RustlsConnect {
    config: TlsConfig,
    domain: String,
}

impl<S> TlsConnect<S> for RustlsConnect
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = RustlsStream<S>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<RustlsStream<S>>> + Send>>;

    fn connect(self, stream: S) -> Self::Future {
        Box::pin(async move { Ok(RustlsStream(self.config.connect(&self.domain, stream).await?)) })
    }
}

/// A TLS stream to PostgreSQL.
pub struct RustlsStream<S>(tokio_rustls::client::TlsStream<S>);

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for RustlsStream<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for RustlsStream<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> tokio_postgres::tls::TlsStream for RustlsStream<S> {
    fn channel_binding(&self) -> ChannelBinding {
        ChannelBinding::none()
    }
}
