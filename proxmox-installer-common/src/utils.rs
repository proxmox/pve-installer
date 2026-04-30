use std::{
    error::Error,
    fmt,
    net::{AddrParseError, IpAddr},
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
    InvalidMask(Box<dyn Error>),
}

impl fmt::Display for CidrAddressParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid CIDR: ")?;

        match self {
            CidrAddressParseError::NoDelimiter => {
                write!(f, "no delimiter for separating address and mask was found")
            }
            CidrAddressParseError::InvalidAddr(err) => write!(f, "{err}"),
            CidrAddressParseError::InvalidMask(err) => write!(f, "{err}"),
        }
    }
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

        check_mask_limit(&addr, mask)?;

        Ok(Self { addr, mask })
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
            .map_err(|err| CidrAddressParseError::InvalidMask(Box::new(err)))?;

        check_mask_limit(&addr, mask)?;

        Ok(Self { addr, mask })
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
        s.parse().map_err(serde::de::Error::custom)
    }
}

serde_plain::derive_serialize_from_display!(CidrAddress);

fn mask_limit(addr: &IpAddr) -> usize {
    if addr.is_ipv4() { 32 } else { 128 }
}

fn check_mask_limit(addr: &IpAddr, mask: usize) -> Result<(), CidrAddressParseError> {
    let limit = mask_limit(addr);
    if mask > limit {
        Err(CidrAddressParseError::InvalidMask(
            format!("mask cannot be greater than {limit}").into(),
        ))
    } else {
        Ok(())
    }
}
