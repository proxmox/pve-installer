use anyhow::Result;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;
use ureq::Agent;
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, Either, LazyBuffers, NextTimeout, TcpConnector,
    Transport, TransportAdapter,
};

/// Builds an [`Agent`] with TLS suitable set up, depending whether a custom fingerprint was
/// supplied or not. If a fingerprint was supplied, only matching certificates will be accepted.
/// Otherwise, the system certificate store is loaded.
///
/// To gather the sha256 fingerprint you can use the following command:
/// ```no_compile
/// openssl s_client -connect <host>:443 < /dev/null 2>/dev/null | openssl x509 -fingerprint -sha256  -noout -in /dev/stdin
/// ```
///
/// # Arguments
/// * `fingerprint` - SHA256 cert fingerprint if certificate pinning should be used. Optional.
fn build_agent(fingerprint: Option<&str>) -> Result<Agent> {
    const GLOBAL_TIMEOUT: Duration = Duration::from_secs(60);

    if let Some(fingerprint) = fingerprint {
        // If the user specified a custom TLS fingerprint, we must use a custom
        // `rustls::ClientConfig`, which in turns means to use a custom
        // `Connector`.
        let crypto_provider = rustls::crypto::CryptoProvider::get_default()
            .cloned()
            .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()));

        let tls_config = ClientConfig::builder_with_provider(crypto_provider)
            .with_protocol_versions(rustls::ALL_VERSIONS)?
            .dangerous()
            .with_custom_certificate_verifier(VerifyCertFingerprint::new(fingerprint)?)
            .with_no_client_auth();

        let connector = UreqRustlsConnector::new(Arc::new(tls_config));

        Ok(Agent::with_parts(
            ureq::config::Config::builder()
                .timeout_global(Some(GLOBAL_TIMEOUT))
                .build(),
            TcpConnector::default().chain(connector),
            DefaultResolver::default(),
        ))
    } else {
        Ok(Agent::config_builder()
            .timeout_global(Some(GLOBAL_TIMEOUT))
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into())
    }
}

/// Issues a GET request to the specified URL and fetches the response. Optionally a SHA256
/// fingerprint can be used to check the certificate against it, instead of the regular certificate
/// validation.
///
/// To gather the sha256 fingerprint you can use the following command:
/// ```no_compile
/// openssl s_client -connect <host>:443 < /dev/null 2>/dev/null | openssl x509 -fingerprint -sha256  -noout -in /dev/stdin
/// ```
///
/// # Arguments
/// * `url` - URL to fetch
/// * `fingerprint` - SHA256 cert fingerprint if certificate pinning should be used. Optional.
/// * `max_size` - Maximum amount of bytes that will be read.
pub fn get_as_bytes(url: &str, fingerprint: Option<&str>, max_size: usize) -> Result<Vec<u8>> {
    let mut result: Vec<u8> = Vec::new();

    let (_, body) = build_agent(fingerprint)?.get(url).call()?.into_parts();

    body.into_reader()
        .take(max_size as u64)
        .read_to_end(&mut result)?;

    Ok(result)
}

/// Issues a POST request with the payload (JSON). Optionally a SHA256 fingerprint can be used to
/// check the cert against it, instead of the regular cert validation.
/// To gather the sha256 fingerprint you can use the following command:
/// ```no_compile
/// openssl s_client -connect <host>:443 < /dev/null 2>/dev/null | openssl x509 -fingerprint -sha256  -noout -in /dev/stdin
/// ```
///
/// # Arguments
/// * `url` - URL to call
/// * `fingerprint` - SHA256 cert fingerprint if certificate pinning should be used. Optional.
/// * `payload` - The payload to send to the server. Expected to be a JSON formatted string.
pub fn post(url: &str, fingerprint: Option<&str>, payload: String) -> Result<String> {
    // TODO: read_to_string limits the size to 10 MB, should be increase that?
    Ok(build_agent(fingerprint)?
        .post(url)
        .header("Content-Type", "application/json; charset=utf-8")
        .send(&payload)?
        .body_mut()
        .read_to_string()?)
}

#[derive(Debug)]
struct VerifyCertFingerprint {
    cert_fingerprint: Vec<u8>,
}

impl VerifyCertFingerprint {
    fn new<S: AsRef<str>>(cert_fingerprint: S) -> Result<std::sync::Arc<Self>> {
        let cert_fingerprint = cert_fingerprint.as_ref();
        let sanitized = cert_fingerprint.replace(':', "");
        let decoded = hex::decode(sanitized)?;
        Ok(std::sync::Arc::new(Self {
            cert_fingerprint: decoded,
        }))
    }
}

impl rustls::client::danger::ServerCertVerifier for VerifyCertFingerprint {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer,
        _intermediates: &[CertificateDer],
        _server_name: &ServerName,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let mut hasher = Sha256::new();
        hasher.update(end_entity);
        let result = hasher.finalize();

        if result.as_slice() == self.cert_fingerprint {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("Fingerprint did not match!".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Mostly a copy of [ureq::unversioned::transport::RustlsConnector], with the exception of using
/// our custom [ClientConfig].
#[derive(Debug)]
struct UreqRustlsConnector {
    /// [ClientConfig] to use for the TLS connection(s).
    config: Arc<ClientConfig>,
}

impl UreqRustlsConnector {
    fn new(config: Arc<ClientConfig>) -> Self {
        UreqRustlsConnector { config }
    }
}

impl<In: Transport> Connector<In> for UreqRustlsConnector {
    type Out = Either<In, UreqRustlsTransport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let Some(transport) = chained else {
            panic!("RustlConnector requires a chained transport");
        };

        if !details.needs_tls() || transport.is_tls() {
            return Ok(Some(Either::A(transport)));
        }

        let name: ServerName<'_> = details
            .uri
            .authority()
            .ok_or(ureq::Error::Tls("no naming authority for URI"))?
            .host()
            .try_into()
            .map_err(|_| ureq::Error::Tls("invalid dns name"))?;

        let conn = ClientConnection::new(self.config.clone(), name.to_owned())?;
        let stream = StreamOwned {
            conn,
            sock: TransportAdapter::new(transport.boxed()),
        };

        let buffers = LazyBuffers::new(
            details.config.input_buffer_size(),
            details.config.output_buffer_size(),
        );

        let transport = UreqRustlsTransport { buffers, stream };

        Ok(Some(Either::B(transport)))
    }
}

/// Direct copy of ureq/tls/rustls.rs:RustlsTransport, which unfortunately is not
/// made public by the crate.
struct UreqRustlsTransport {
    buffers: LazyBuffers,
    stream: StreamOwned<ClientConnection, TransportAdapter>,
}

impl Transport for UreqRustlsTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.stream.get_mut().set_timeout(timeout);

        let output = &self.buffers.output()[..amount];
        self.stream.write_all(output)?;

        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.stream.get_mut().set_timeout(timeout);

        let input = self.buffers.input_append_buf();
        let amount = self.stream.read(input)?;
        self.buffers.input_appended(amount);

        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        self.stream.get_mut().get_mut().is_open()
    }

    fn is_tls(&self) -> bool {
        true
    }
}

impl fmt::Debug for UreqRustlsTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RustlsTransport")
            .field("chained", &self.stream.sock.inner())
            .field("buffers", &self.buffers)
            .finish()
    }
}
