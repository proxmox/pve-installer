use anyhow::{bail, Result};
use log::info;
use serde::Serialize;
use std::{
    fs::{self, read_to_string},
    process::Command,
};

use proxmox_auto_installer::{sysinfo::SysInfo, utils::HttpOptions};

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

/// Metadata of the HTTP POST payload, such as schema version of the document.
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct HttpFetchInfoSchema {
    /// major.minor version describing the schema version of this document, in a semanticy-version
    /// way.
    ///
    /// major: Incremented for incompatible/breaking API changes, e.g. removing an existing
    /// field.
    /// minor: Incremented when adding functionality in a backwards-compatible matter, e.g.
    /// adding a new field.
    version: String,
}

impl HttpFetchInfoSchema {
    const SCHEMA_VERSION: &str = "1.0";
}

impl Default for HttpFetchInfoSchema {
    fn default() -> Self {
        Self {
            version: Self::SCHEMA_VERSION.to_owned(),
        }
    }
}

/// All data sent as request payload with the answerfile fetch POST request.
///
/// NOTE: The format is versioned through `schema.version` (`$schema.version` in the
/// resulting JSON), ensure you update it when this struct or any of its members gets modified.
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct HttpFetchPayload {
    /// Metadata for the answerfile fetch payload
    // This field is prefixed by `$` on purpose, to indicate that it is document metadata and not
    // part of the actual content itself. (E.g. JSON Schema uses a similar naming scheme)
    #[serde(rename = "$schema")]
    schema: HttpFetchInfoSchema,
    /// Information about the running system, flattened into this structure directly.
    #[serde(flatten)]
    sysinfo: SysInfo,
}

impl HttpFetchPayload {
    /// Retrieves the required information from the system and constructs the
    /// full payload including meta data.
    fn get() -> Result<Self> {
        Ok(Self {
            schema: HttpFetchInfoSchema::default(),
            sysinfo: SysInfo::get()?,
        })
    }

    /// Retrieves the required information from the system and constructs the
    /// full payload including meta data, serialized as JSON.
    pub fn as_json() -> Result<String> {
        let info = Self::get()?;
        Ok(serde_json::to_string(&info)?)
    }
}

pub struct FetchFromHTTP;

impl FetchFromHTTP {
    /// Will try to fetch the answer.toml by sending a HTTP POST request. The URL can be configured
    /// either via DHCP or DNS or preconfigured in the ISO.
    /// If the URL is not defined in the ISO, it will first check DHCP options. The SSL certificate
    /// needs to be either trusted by the root certs or a SHA256 fingerprint needs to be provided.
    /// The SHA256 SSL fingerprint can either be defined in the ISO, as DHCP option, or as DNS TXT
    /// record. If provided, the fingerprint provided in the ISO has preference.
    pub fn get_answer(settings: &HttpOptions) -> Result<String> {
        let mut fingerprint: Option<String> = match settings.cert_fingerprint.clone() {
            Some(fp) => {
                info!("SSL fingerprint provided through ISO.");
                Some(fp)
            }
            None => None,
        };

        let answer_url: String;
        if let Some(url) = settings.url.clone() {
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
        let payload = HttpFetchPayload::as_json()?;

        info!("Sending POST request to '{answer_url}'.");
        let answer =
            proxmox_installer_common::http::post(&answer_url, fingerprint.as_deref(), payload)?;
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
