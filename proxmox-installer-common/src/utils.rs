use std::{
    fmt,
    net::{AddrParseError, IpAddr},
    num::ParseIntError,
    str::FromStr,
};

use serde::Deserialize;

/// Possible errors that might occur when parsing CIDR addresses.
#[derive(Debug)]
pub enum CidrAddressParseError {
    /// No delimiter for separating address and mask was found.
    NoDelimiter,
    /// The IP address part could not be parsed.
    InvalidAddr(AddrParseError),
    /// The mask could not be parsed.
    InvalidMask(Option<ParseIntError>),
}

/// An IP address (IPv4 or IPv6), including network mask.
///
/// See the [`IpAddr`] type for more information how IP addresses are handled.
/// The mask is appropriately enforced to be `0 <= mask <= 32` for IPv4 or
/// `0 <= mask <= 128` for IPv6 addresses.
///
/// # Examples
/// ```
/// use std::net::{Ipv4Addr, Ipv6Addr};
/// use proxmox_installer_common::utils::CidrAddress;
/// let ipv4 = CidrAddress::new(Ipv4Addr::new(192, 168, 0, 1), 24).unwrap();
/// let ipv6 = CidrAddress::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0xc0a8, 1), 32).unwrap();
///
/// assert_eq!(ipv4.to_string(), "192.168.0.1/24");
/// assert_eq!(ipv6.to_string(), "2001:db8::c0a8:1/32");
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct CidrAddress {
    addr: IpAddr,
    mask: usize,
}

impl CidrAddress {
    /// Constructs a new CIDR address.
    ///
    /// It fails if the mask is invalid for the given IP address.
    pub fn new<T: Into<IpAddr>>(addr: T, mask: usize) -> Result<Self, CidrAddressParseError> {
        let addr = addr.into();

        if mask > mask_limit(&addr) {
            Err(CidrAddressParseError::InvalidMask(None))
        } else {
            Ok(Self { addr, mask })
        }
    }

    /// Returns only the IP address part of the address.
    pub fn addr(&self) -> IpAddr {
        self.addr
    }

    /// Returns `true` if this address is an IPv4 address, `false` otherwise.
    pub fn is_ipv4(&self) -> bool {
        self.addr.is_ipv4()
    }

    /// Returns `true` if this address is an IPv6 address, `false` otherwise.
    pub fn is_ipv6(&self) -> bool {
        self.addr.is_ipv6()
    }

    /// Returns only the mask part of the address.
    pub fn mask(&self) -> usize {
        self.mask
    }
}

impl FromStr for CidrAddress {
    type Err = CidrAddressParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr, mask) = s
            .split_once('/')
            .ok_or(CidrAddressParseError::NoDelimiter)?;

        let addr = addr.parse().map_err(CidrAddressParseError::InvalidAddr)?;

        let mask = mask
            .parse()
            .map_err(|err| CidrAddressParseError::InvalidMask(Some(err)))?;

        if mask > mask_limit(&addr) {
            Err(CidrAddressParseError::InvalidMask(None))
        } else {
            Ok(Self { addr, mask })
        }
    }
}

impl fmt::Display for CidrAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.mask)
    }
}

impl<'de> Deserialize<'de> for CidrAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        s.parse()
            .map_err(|_| serde::de::Error::custom("invalid CIDR"))
    }
}

serde_plain::derive_serialize_from_display!(CidrAddress);

fn mask_limit(addr: &IpAddr) -> usize {
    if addr.is_ipv4() {
        32
    } else {
        128
    }
}

/// Possible errors that might occur when parsing FQDNs.
#[derive(Debug, Eq, PartialEq)]
pub enum FqdnParseError {
    MissingHostname,
    NumericHostname,
    InvalidPart(String),
    TooLong(usize),
}

impl fmt::Display for FqdnParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use FqdnParseError::*;
        match self {
            MissingHostname => write!(f, "missing hostname part"),
            NumericHostname => write!(f, "hostname cannot be purely numeric"),
            InvalidPart(part) => write!(
                f,
                "FQDN must only consist of alphanumeric characters and dashes. Invalid part: '{part}'",
            ),
            TooLong(len) => write!(f, "FQDN too long: {len} > {}", Fqdn::MAX_LENGTH),
        }
    }
}

/// A type for safely representing fully-qualified domain names (FQDNs).
///
/// It considers following RFCs:
/// - [RFC952] (sec. "ASSUMPTIONS", 1.)
/// - [RFC1035] (sec. 2.3. "Conventions")
/// - [RFC1123] (sec. 2.1. "Host Names and Numbers")
/// - [RFC3492]
/// - [RFC4343]
///
/// .. and applies some restriction given by Debian, e.g. 253 instead of 255
/// maximum total length and maximum 63 characters per label, per the
/// [hostname(7)].
///
/// Additionally:
/// - It enforces the restriction as per Bugzilla #1054, in that
///   purely numeric hostnames are not allowed - against RFC1123 sec. 2.1.
///
/// Some terminology:
/// - "label" - a single part of a FQDN, e.g. {label}.{label}.{tld}
///
/// [RFC952]: <https://www.ietf.org/rfc/rfc952.txt>
/// [RFC1035]: <https://www.ietf.org/rfc/rfc1035.txt>
/// [RFC1123]: <https://www.ietf.org/rfc/rfc1123.txt>
/// [RFC3492]: <https://www.ietf.org/rfc/rfc3492.txt>
/// [RFC4343]: <https://www.ietf.org/rfc/rfc4343.txt>
/// [hostname(7)]: <https://manpages.debian.org/stable/manpages/hostname.7.en.html>
#[derive(Clone, Debug, Eq)]
pub struct Fqdn {
    parts: Vec<String>,
}

impl Fqdn {
    /// Maximum length of a single label of the FQDN
    const MAX_LABEL_LENGTH: usize = 63;
    /// Maximum total length of the FQDN
    const MAX_LENGTH: usize = 253;

    pub fn from(fqdn: &str) -> Result<Self, FqdnParseError> {
        if fqdn.len() > Self::MAX_LENGTH {
            return Err(FqdnParseError::TooLong(fqdn.len()));
        }

        let parts = fqdn
            .split('.')
            .map(ToOwned::to_owned)
            .collect::<Vec<String>>();

        for part in &parts {
            if !Self::validate_single(part) {
                return Err(FqdnParseError::InvalidPart(part.clone()));
            }
        }

        if parts.len() < 2 {
            Err(FqdnParseError::MissingHostname)
        } else if parts[0].chars().all(|c| c.is_ascii_digit()) {
            // Do not allow a purely numeric hostname, see:
            // https://bugzilla.proxmox.com/show_bug.cgi?id=1054
            Err(FqdnParseError::NumericHostname)
        } else {
            Ok(Self { parts })
        }
    }

    pub fn host(&self) -> Option<&str> {
        self.has_host().then_some(&self.parts[0])
    }

    pub fn domain(&self) -> String {
        let parts = if self.has_host() {
            &self.parts[1..]
        } else {
            &self.parts
        };

        parts.join(".")
    }

    /// Checks whether the FQDN has a hostname associated with it, i.e. is has more than 1 part.
    fn has_host(&self) -> bool {
        self.parts.len() > 1
    }

    fn validate_single(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= Self::MAX_LABEL_LENGTH
            // First character must be alphanumeric
            && s.chars()
                .next()
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or_default()
            // .. last character as well,
            && s.chars()
                .last()
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or_default()
            // and anything between must be alphanumeric or -
            && s.chars()
                .skip(1)
                .take(s.len().saturating_sub(2))
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
    }
}

impl FromStr for Fqdn {
    type Err = FqdnParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from(value)
    }
}

impl fmt::Display for Fqdn {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.parts.join("."))
    }
}

impl<'de> Deserialize<'de> for Fqdn {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        s.parse()
            .map_err(|_| serde::de::Error::custom("invalid FQDN"))
    }
}

impl PartialEq for Fqdn {
    // Case-insensitive comparison, as per RFC 952 "ASSUMPTIONS", RFC 1035 sec. 2.3.3. "Character
    // Case" and RFC 4343 as a whole
    fn eq(&self, other: &Self) -> bool {
        if self.parts.len() != other.parts.len() {
            return false;
        }

        self.parts
            .iter()
            .zip(other.parts.iter())
            .all(|(a, b)| a.to_lowercase() == b.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fqdn_construct() {
        use FqdnParseError::*;
        assert!(Fqdn::from("foo.example.com").is_ok());
        assert!(Fqdn::from("foo-bar.com").is_ok());
        assert!(Fqdn::from("a-b.com").is_ok());

        assert_eq!(Fqdn::from("foo"), Err(MissingHostname));

        assert_eq!(Fqdn::from("-foo.com"), Err(InvalidPart("-foo".to_owned())));
        assert_eq!(Fqdn::from("foo-.com"), Err(InvalidPart("foo-".to_owned())));
        assert_eq!(Fqdn::from("foo.com-"), Err(InvalidPart("com-".to_owned())));
        assert_eq!(Fqdn::from("-o-.com"), Err(InvalidPart("-o-".to_owned())));

        // https://bugzilla.proxmox.com/show_bug.cgi?id=1054
        assert_eq!(Fqdn::from("123.com"), Err(NumericHostname));
        assert!(Fqdn::from("foo123.com").is_ok());
        assert!(Fqdn::from("123foo.com").is_ok());

        assert!(Fqdn::from(&format!("{}.com", "a".repeat(63))).is_ok());
        assert_eq!(
            Fqdn::from(&format!("{}.com", "a".repeat(250))),
            Err(TooLong(254)),
        );
        assert_eq!(
            Fqdn::from(&format!("{}.com", "a".repeat(64))),
            Err(InvalidPart("a".repeat(64))),
        );

        // https://bugzilla.proxmox.com/show_bug.cgi?id=5230
        assert_eq!(
            Fqdn::from("123@foo.com"),
            Err(InvalidPart("123@foo".to_owned()))
        );
    }

    #[test]
    fn fqdn_parts() {
        let fqdn = Fqdn::from("pve.example.com").unwrap();
        assert_eq!(fqdn.host().unwrap(), "pve");
        assert_eq!(fqdn.domain(), "example.com");
        assert_eq!(
            fqdn.parts,
            &["pve".to_owned(), "example".to_owned(), "com".to_owned()]
        );
    }

    #[test]
    fn fqdn_display() {
        assert_eq!(
            Fqdn::from("foo.example.com").unwrap().to_string(),
            "foo.example.com"
        );
    }

    #[test]
    fn fqdn_compare() {
        assert_eq!(Fqdn::from("example.com"), Fqdn::from("example.com"));
        assert_eq!(Fqdn::from("example.com"), Fqdn::from("ExAmPle.Com"));
        assert_eq!(Fqdn::from("ExAmPle.Com"), Fqdn::from("example.com"));
        assert_ne!(
            Fqdn::from("subdomain.ExAmPle.Com"),
            Fqdn::from("example.com")
        );
        assert_ne!(Fqdn::from("foo.com"), Fqdn::from("bar.com"));
        assert_ne!(Fqdn::from("example.com"), Fqdn::from("example.net"));
    }
}
