use anyhow::{bail, Result};
use log::info;
use std::{
    fs::{self, read_to_string},
    process::Command,
};

use proxmox_auto_installer::{sysinfo::SysInfo, utils::AutoInstSettings};

static ANSWER_URL_SUBDOMAIN: &str = "proxmox-auto-installer";
static ANSWER_CERT_FP_SUBDOMAIN: &str = "proxmox-auto-installer-cert-fingerprint";

// It is possible to set custom DHPC options. Option numbers 224 to 254 [0].
// To use them with dhclient, we need to configure it to request them and what they should be
// called.
//
// e.g. /etc/dhcp/dhclient.conf:
// ```
// option proxmox-auto-installer-manifest-url code 250 = text;
// option proxmox-auto-installer-cert-fingerprint code 251 = text;
// also request proxmox-auto-installer-manifest-url, proxmox-auto-installer-cert-fingerprint;
// ```
//
// The results will end up in the /var/lib/dhcp/dhclient.leases file from where we can fetch them
//
// [0] https://www.iana.org/assignments/bootp-dhcp-parameters/bootp-dhcp-parameters.xhtml
static DHCP_URL_OPTION: &str = "proxmox-auto-installer-manifest-url";
static DHCP_CERT_FP_OPTION: &str = "proxmox-auto-installer-cert-fingerprint";
static DHCP_LEASE_FILE: &str = "/var/lib/dhcp/dhclient.leases";

pub struct FetchFromHTTP;

impl FetchFromHTTP {
    /// Will try to fetch the answer.toml by sending a HTTP POST request. The URL can be configured
    /// either via DHCP or DNS or preconfigured in the ISO.
    /// If the URL is not defined in the ISO, it will first check DHCP options. The SSL certificate
    /// needs to be either trusted by the root certs or a SHA256 fingerprint needs to be provided.
    /// The SHA256 SSL fingerprint can either be defined in the ISO, as DHCP option, or as DNS TXT
    /// record. If provided, the fingerprint provided in the ISO has preference.
    pub fn get_answer(settings: &AutoInstSettings) -> Result<String> {
        let mut fingerprint: Option<String> = match settings.cert_fingerprint.clone() {
            Some(fp) => {
                info!("SSL fingerprint provided through ISO.");
                Some(fp)
            }
            None => None,
        };

        let answer_url: String;
        if let Some(url) = settings.http_url.clone() {
            info!("URL specified in ISO");
            answer_url = url;
        } else {
            (answer_url, fingerprint) = match Self::fetch_dhcp(fingerprint.clone()) {
                Ok((url, fp)) => (url, fp),
                Err(err) => {
                    info!("{err}");
                    Self::fetch_dns(fingerprint.clone())?
                }
            };
        }

        if let Some(fingerprint) = &fingerprint {
            let _ = fs::write("/tmp/cert_fingerprint", fingerprint);
        }

        info!("Gathering system information.");
        let payload = SysInfo::as_json()?;
        info!("Sending POST request to '{answer_url}'.");
        let answer = http_post::call(answer_url, fingerprint.as_deref(), payload)?;
        Ok(answer)
    }

    /// Fetches search domain from resolv.conf file
    fn get_search_domain() -> Result<String> {
        info!("Retrieving default search domain.");
        for line in read_to_string("/etc/resolv.conf")?.lines() {
            if let Some((key, value)) = line.split_once(' ') {
                if key == "search" {
                    return Ok(value.trim().into());
                }
            }
        }
        bail!("Could not find search domain in resolv.conf.");
    }

    /// Runs a TXT DNS query on the domain provided
    fn query_txt_record(query: String) -> Result<String> {
        info!("Querying TXT record for '{query}'");
        let url: String;
        match Command::new("dig")
            .args(["txt", "+short"])
            .arg(&query)
            .output()
        {
            Ok(output) => {
                if output.status.success() {
                    url = String::from_utf8(output.stdout)?
                        .replace('"', "")
                        .trim()
                        .into();
                    if url.is_empty() {
                        bail!("Got empty response.");
                    }
                } else {
                    bail!(
                        "Error querying DNS record '{query}' : {}",
                        String::from_utf8(output.stderr)?
                    );
                }
            }
            Err(err) => bail!("Error querying DNS record '{query}': {err}"),
        }
        info!("Found: '{url}'");
        Ok(url)
    }

    /// Tries to fetch answer URL and SSL fingerprint info from DNS
    fn fetch_dns(mut fingerprint: Option<String>) -> Result<(String, Option<String>)> {
        let search_domain = Self::get_search_domain()?;

        let answer_url =
            match Self::query_txt_record(format!("{ANSWER_URL_SUBDOMAIN}.{search_domain}")) {
                Ok(url) => url,
                Err(err) => bail!("{err}"),
            };

        if fingerprint.is_none() {
            fingerprint =
                match Self::query_txt_record(format!("{ANSWER_CERT_FP_SUBDOMAIN}.{search_domain}"))
                {
                    Ok(fp) => Some(fp),
                    Err(err) => {
                        info!("{err}");
                        None
                    }
                };
        }
        Ok((answer_url, fingerprint))
    }

    /// Tries to fetch answer URL and SSL fingerprint info from DHCP options
    fn fetch_dhcp(mut fingerprint: Option<String>) -> Result<(String, Option<String>)> {
        info!("Checking DHCP options.");
        let leases = fs::read_to_string(DHCP_LEASE_FILE)?;

        let mut answer_url: Option<String> = None;

        let url_match = format!("option {DHCP_URL_OPTION}");
        let fp_match = format!("option {DHCP_CERT_FP_OPTION}");

        for line in leases.lines() {
            if answer_url.is_none() && line.trim().starts_with(url_match.as_str()) {
                answer_url = Self::strip_dhcp_option(line.split(' ').nth_back(0));
            }
            if fingerprint.is_none() && line.trim().starts_with(fp_match.as_str()) {
                fingerprint = Self::strip_dhcp_option(line.split(' ').nth_back(0));
            }
        }

        let answer_url = match answer_url {
            None => bail!("No DHCP option found for fetch URL."),
            Some(url) => {
                info!("Found URL for answer in DHCP option: '{url}'");
                url
            }
        };

        if let Some(fp) = fingerprint.clone() {
            info!("Found SSL Fingerprint via DHCP: '{fp}'");
        }

        Ok((answer_url, fingerprint))
    }

    /// Clean DHCP option string
    fn strip_dhcp_option(value: Option<&str>) -> Option<String> {
        // value is expected to be in format: "value";
        value.map(|value| String::from(&value[1..value.len() - 2]))
    }
}

mod http_post {
    use anyhow::Result;
    use rustls::ClientConfig;
    use sha2::{Digest, Sha256};
    use std::sync::Arc;
    use ureq::{Agent, AgentBuilder};

    /// Issues a POST request with the payload (JSON). Optionally a SHA256 fingerprint can be used to
    /// check the cert against it, instead of the regular cert validation.
    /// To gather the sha256 fingerprint you can use the following command:
    /// ```no_compile
    /// openssl s_client -connect <host>:443 < /dev/null 2>/dev/null | openssl x509 -fingerprint -sha256  -noout -in /dev/stdin
    /// ```
    ///
    /// # Arguemnts
    /// * `url` - URL to call
    /// * `fingerprint` - SHA256 cert fingerprint if certificate pinning should be used. Optional.
    /// * `payload` - The payload to send to the server. Expected to be a JSON formatted string.
    pub fn call(url: String, fingerprint: Option<&str>, payload: String) -> Result<String> {
        let answer;

        if let Some(fingerprint) = fingerprint {
            let tls_config = ClientConfig::builder()
                .with_safe_defaults()
                .with_custom_certificate_verifier(VerifyCertFingerprint::new(fingerprint)?)
                .with_no_client_auth();

            let agent: Agent = AgentBuilder::new().tls_config(Arc::new(tls_config)).build();

            answer = agent
                .post(&url)
                .set("Content-type", "application/json; charset=utf-")
                .send_string(&payload)?
                .into_string()?;
        } else {
            let mut roots = rustls::RootCertStore::empty();
            for cert in rustls_native_certs::load_native_certs()? {
                roots.add(&rustls::Certificate(cert.0)).unwrap();
            }

            let tls_config = rustls::ClientConfig::builder()
                .with_safe_defaults()
                .with_root_certificates(roots)
                .with_no_client_auth();

            let agent = AgentBuilder::new()
                .tls_connector(Arc::new(native_tls::TlsConnector::new()?))
                .tls_config(Arc::new(tls_config))
                .build();
            answer = agent
                .post(&url)
                .set("Content-type", "application/json; charset=utf-")
                .timeout(std::time::Duration::from_secs(60))
                .send_string(&payload)?
                .into_string()?;
        }
        Ok(answer)
    }

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

    impl rustls::client::ServerCertVerifier for VerifyCertFingerprint {
        fn verify_server_cert(
            &self,
            end_entity: &rustls::Certificate,
            _intermediates: &[rustls::Certificate],
            _server_name: &rustls::ServerName,
            _scts: &mut dyn Iterator<Item = &[u8]>,
            _ocsp_response: &[u8],
            _now: std::time::SystemTime,
        ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
            let mut hasher = Sha256::new();
            hasher.update(end_entity);
            let result = hasher.finalize();

            if result.as_slice() == self.cert_fingerprint {
                Ok(rustls::client::ServerCertVerified::assertion())
            } else {
                Err(rustls::Error::General("Fingerprint did not match!".into()))
            }
        }
    }
}
