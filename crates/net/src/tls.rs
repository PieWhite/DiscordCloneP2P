//! QUIC vereist TLS. Wij gebruiken het niet voor vertrouwen.
//!
//! Het verkeer loopt al door WireGuard (Tailscale), dus vertrouwelijkheid en
//! integriteit zijn daar al geregeld. Deze laag bestaat puur omdat QUIC niet zonder
//! kan. Elke peer genereert bij het starten een wegwerp-certificaat en accepteert dat
//! van de ander zonder verificatie.
//!
//! Authenticatie gebeurt daarna op applicatieniveau: alleen peers waarvan het `PeerId`
//! overeenkomt met de configuratie, of die vanaf een geconfigureerd tailnet-adres
//! komen, worden toegelaten. Het tailnet is de echte beveiligingsgrens.
//!
//! Zie `TODO.md` als je dit ooit wilt aanscherpen met certificaat-pinning.

use anyhow::{Context, Result};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::sync::Arc;
use std::time::Duration;

/// Vaste SNI-naam. Betekenisloos omdat we niets verifiëren, maar QUIC eist een naam.
pub const SERVER_NAME: &str = "fitcom";

/// Sneller dan wachten op een TCP-achtige timeout, maar ruim genoeg om een korte
/// wifi-hik te overleven zonder de verbinding weg te gooien.
const KEEP_ALIVE: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(15);

fn transport_config() -> quinn::TransportConfig {
    let mut t = quinn::TransportConfig::default();
    t.keep_alive_interval(Some(KEEP_ALIVE));
    t.max_idle_timeout(Some(
        IDLE_TIMEOUT
            .try_into()
            .expect("idle timeout past in varint"),
    ));
    t
}

pub fn server_config() -> Result<quinn::ServerConfig> {
    let cert = rcgen::generate_simple_self_signed(vec![SERVER_NAME.to_string()])
        .context("zelfondertekend certificaat genereren")?;
    let key = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let chain = vec![CertificateDer::from(cert.cert)];

    let mut crypto = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_no_client_auth()
    .with_single_cert(chain, key.into())
    .context("certificaat aan rustls koppelen")?;
    crypto.alpn_protocols = vec![b"fitcom/1".to_vec()];

    let mut cfg = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(crypto)?));
    cfg.transport_config(Arc::new(transport_config()));
    Ok(cfg)
}

pub fn client_config() -> Result<quinn::ClientConfig> {
    let mut crypto = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(AcceptAnyCertificate))
    .with_no_client_auth();
    crypto.alpn_protocols = vec![b"fitcom/1".to_vec()];

    let mut cfg = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(crypto)?));
    cfg.transport_config(Arc::new(transport_config()));
    Ok(cfg)
}

/// Accepteert elk certificaat. Zie de moduledocumentatie voor waarom dat hier mag.
#[derive(Debug)]
struct AcceptAnyCertificate;

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
