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
/// let ipv4 = CidrAddress::new(Ipv4Addr::new(192, 168, 0, 1), 24).unwrap();
/// let ipv6 = CidrAddress::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0xc0a8, 1), 32).unwrap();
///
/// assert_eq!(ipv4.to_string(), "192.168.0.1/24");
/// assert_eq!(ipv6.to_string(), "2001:db8::c0a8:1/32");
/// ```
#[derive(Clone, Debug)]
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
        self.addr.is_ipv4()
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

fn mask_limit(addr: &IpAddr) -> usize {
    if addr.is_ipv4() {
        32
    } else {
        128
    }
}

#[derive(Clone, Debug)]
pub struct Fqdn {
    host: String,
    domain: String,
}

impl Fqdn {
    pub fn from(fqdn: &str) -> Result<Self, ()> {
        let (host, domain) = fqdn.split_once('.').ok_or(())?;

        if !Self::validate_single(host) || !domain.split('.').all(Self::validate_single) {
            Err(())
        } else {
            Ok(Self {
                host: host.to_owned(),
                domain: domain.to_owned(),
            })
        }
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn domain(&self) -> &str {
        &self.domain
    }

    fn validate_single(s: &str) -> bool {
        !s.is_empty()
            && s.chars()
                .next()
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or_default()
            && s.chars()
                .last()
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or_default()
            && s.chars()
                .skip(1)
                .take(s.len().saturating_sub(2))
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
    }
}

impl FromStr for Fqdn {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from(value)
    }
}

impl fmt::Display for Fqdn {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{}", self.host, self.domain)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fqdn_validate_single() {
        assert!(Fqdn::from("foo.example.com").is_ok());
        assert!(Fqdn::from("1.example.com").is_ok());
        assert!(Fqdn::from("foo-bar.com").is_ok());
        assert!(Fqdn::from("a-b.com").is_ok());

        assert!(Fqdn::from("foo").is_err());
        assert!(Fqdn::from("-foo.com").is_err());
        assert!(Fqdn::from("foo-.com").is_err());
        assert!(Fqdn::from("foo.com-").is_err());
        assert!(Fqdn::from("-o-.com").is_err());
    }

    #[test]
    fn fqdn_parts() {
        let fqdn = Fqdn::from("pve.example.com").unwrap();
        assert_eq!(fqdn.host(), "pve");
        assert_eq!(fqdn.domain(), "example.com");
    }

    #[test]
    fn fqdn_display() {
        assert_eq!(
            Fqdn::from("foo.example.com").unwrap().to_string(),
            "foo.example.com"
        );
    }
}
