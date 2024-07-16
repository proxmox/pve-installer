use std::{
    cmp,
    collections::{BTreeMap, HashMap},
    fmt,
    fs::File,
    io::{self, BufReader},
    net::IpAddr,
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
};

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    options::{
        BtrfsRaidLevel, Disk, FsType, ZfsBootdiskOptions, ZfsChecksumOption, ZfsCompressOption,
        ZfsRaidLevel,
    },
    utils::CidrAddress,
};

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxmoxProduct {
    PVE,
    PBS,
    PMG,
}

impl ProxmoxProduct {
    pub fn default_hostname(self) -> &'static str {
        match self {
            Self::PVE => "pve",
            Self::PMG => "pmg",
            Self::PBS => "pbs",
        }
    }
}

impl fmt::Display for ProxmoxProduct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PVE => "pve",
            Self::PMG => "pmg",
            Self::PBS => "pbs",
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProductConfig {
    pub fullname: String,
    pub product: ProxmoxProduct,
    #[serde(deserialize_with = "deserialize_bool_from_int")]
    pub enable_btrfs: bool,
}

impl ProductConfig {
    /// A mocked ProductConfig simulating a Proxmox VE environment.
    pub fn mocked() -> Self {
        Self {
            fullname: String::from("Proxmox VE (mocked)"),
            product: ProxmoxProduct::PVE,
            enable_btrfs: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IsoInfo {
    pub release: String,
    pub isorelease: String,
}

impl IsoInfo {
    /// A mocked IsoInfo with some edge case to convey that this is not necessarily purely numeric.
    pub fn mocked() -> Self {
        Self {
            release: String::from("42.1"),
            isorelease: String::from("mocked-1"),
        }
    }
}

/// Paths in the ISO environment containing installer data.
#[derive(Clone, Deserialize)]
pub struct IsoLocations {
    pub iso: PathBuf,
}

impl IsoLocations {
    /// A mocked location, uses the current working directory by default
    pub fn mocked() -> Self {
        Self {
            iso: std::env::current_dir().unwrap_or("/dev/null".into()),
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct SetupInfo {
    #[serde(rename = "product-cfg")]
    pub config: ProductConfig,
    #[serde(rename = "iso-info")]
    pub iso_info: IsoInfo,
    pub locations: IsoLocations,
}

impl SetupInfo {
    /// Return a mocked SetupInfo that is very similar to how our actual ones look like and should
    /// be good enough for testing.
    pub fn mocked() -> Self {
        Self {
            config: ProductConfig::mocked(),
            iso_info: IsoInfo::mocked(),
            locations: IsoLocations::mocked(),
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct CountryInfo {
    pub name: String,
    #[serde(default)]
    pub zone: String,
    pub kmap: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq)]
pub struct KeyboardMapping {
    pub name: String,
    #[serde(rename = "kvm")]
    pub id: String,
    #[serde(rename = "x11")]
    pub xkb_layout: String,
    #[serde(rename = "x11var")]
    pub xkb_variant: String,
}

impl cmp::PartialOrd for KeyboardMapping {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl cmp::Ord for KeyboardMapping {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

#[derive(Clone, Deserialize)]
pub struct LocaleInfo {
    #[serde(deserialize_with = "deserialize_cczones_map")]
    pub cczones: HashMap<String, Vec<String>>,
    #[serde(rename = "country")]
    pub countries: HashMap<String, CountryInfo>,
    pub kmap: HashMap<String, KeyboardMapping>,
}

/// Fetches basic information needed for the installer which is required to work
pub fn installer_setup(in_test_mode: bool) -> Result<(SetupInfo, LocaleInfo, RuntimeInfo), String> {
    let base_path = if in_test_mode {
        format!("./testdir/{}", crate::RUNTIME_DIR)
    } else {
        crate::RUNTIME_DIR.to_owned()
    };
    let path = PathBuf::from(base_path);

    let installer_info: SetupInfo = {
        let mut path = path.clone();
        path.push("iso-info.json");

        read_json(&path).map_err(|err| format!("Failed to retrieve setup info: {err}"))?
    };

    let locale_info = {
        let mut path = path.clone();
        path.push("locales.json");

        read_json(&path).map_err(|err| format!("Failed to retrieve locale info: {err}"))?
    };

    let mut runtime_info: RuntimeInfo = {
        let mut path = path.clone();
        path.push("run-env-info.json");

        read_json(&path)
            .map_err(|err| format!("Failed to retrieve runtime environment info: {err}"))?
    };

    runtime_info.disks.sort();
    if runtime_info.disks.is_empty() {
        Err("The installer could not find any supported hard disks.".to_owned())
    } else {
        Ok((installer_info, locale_info, runtime_info))
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct InstallZfsOption {
    pub ashift: usize,
    #[serde(serialize_with = "serialize_as_display")]
    pub compress: ZfsCompressOption,
    #[serde(serialize_with = "serialize_as_display")]
    pub checksum: ZfsChecksumOption,
    pub copies: usize,
    pub arc_max: usize,
}

impl From<ZfsBootdiskOptions> for InstallZfsOption {
    fn from(opts: ZfsBootdiskOptions) -> Self {
        InstallZfsOption {
            ashift: opts.ashift,
            compress: opts.compress,
            checksum: opts.checksum,
            copies: opts.copies,
            arc_max: opts.arc_max,
        }
    }
}

pub fn read_json<T: for<'de> Deserialize<'de>, P: AsRef<Path>>(path: P) -> Result<T, String> {
    let file = File::open(path).map_err(|err| err.to_string())?;
    let reader = BufReader::new(file);

    serde_json::from_reader(reader).map_err(|err| format!("failed to parse JSON: {err}"))
}

fn deserialize_bool_from_int<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let val: u32 = Deserialize::deserialize(deserializer)?;
    Ok(val != 0)
}

fn deserialize_cczones_map<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    let map: HashMap<String, HashMap<String, u32>> = Deserialize::deserialize(deserializer)?;

    let mut result = HashMap::new();
    for (cc, list) in map.into_iter() {
        result.insert(cc, list.into_keys().collect());
    }

    Ok(result)
}

fn deserialize_disks_map<'de, D>(deserializer: D) -> Result<Vec<Disk>, D::Error>
where
    D: Deserializer<'de>,
{
    let disks =
        <Vec<(usize, String, f64, String, Option<usize>, String)>>::deserialize(deserializer)?;
    Ok(disks
        .into_iter()
        .map(
            |(index, device, size_mb, model, logical_bsize, _syspath)| Disk {
                index: index.to_string(),
                // Linux always reports the size of block devices in sectors, where one sector is
                // defined as being 2^9 = 512 bytes in size.
                // https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/tree/include/linux/blk_types.h?h=v6.4#n30
                size: (size_mb * 512.) / 1024. / 1024. / 1024.,
                block_size: logical_bsize,
                path: device,
                model: (!model.is_empty()).then_some(model),
            },
        )
        .collect())
}

fn deserialize_cidr_list<'de, D>(deserializer: D) -> Result<Option<Vec<CidrAddress>>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct CidrDescriptor {
        address: String,
        prefix: usize,
        // family is implied anyway by parsing the address
    }

    let list: Vec<CidrDescriptor> = Deserialize::deserialize(deserializer)?;

    let mut result = Vec::with_capacity(list.len());
    for desc in list {
        let ip_addr = desc
            .address
            .parse::<IpAddr>()
            .map_err(|err| de::Error::custom(format!("{:?}", err)))?;

        result.push(
            CidrAddress::new(ip_addr, desc.prefix)
                .map_err(|err| de::Error::custom(format!("{:?}", err)))?,
        );
    }

    Ok(Some(result))
}

fn serialize_as_display<S, T>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: fmt::Display,
{
    serializer.collect_str(value)
}

#[derive(Clone, Deserialize)]
pub struct RuntimeInfo {
    /// Whether is system was booted in (legacy) BIOS or UEFI mode.
    pub boot_type: BootType,

    /// Detected country if available.
    pub country: Option<String>,

    /// Maps devices to their information.
    #[serde(deserialize_with = "deserialize_disks_map")]
    pub disks: Vec<Disk>,

    /// Network addresses, gateways and DNS info.
    pub network: NetworkInfo,

    /// Total memory of the system in MiB.
    pub total_memory: usize,

    /// Whether the CPU supports hardware-accelerated virtualization
    #[serde(deserialize_with = "deserialize_bool_from_int")]
    pub hvm_supported: bool,
}

#[derive(Copy, Clone, Eq, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BootType {
    Bios,
    Efi,
}

#[derive(Clone, Deserialize)]
pub struct NetworkInfo {
    pub dns: Dns,
    pub routes: Option<Routes>,

    /// Maps devices to their configuration, if it has a usable configuration.
    /// (Contains no entries for devices with only link-local addresses.)
    #[serde(default)]
    pub interfaces: BTreeMap<String, Interface>,

    /// The hostname of this machine, if set by the DHCP server.
    pub hostname: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct Dns {
    pub domain: Option<String>,

    /// List of stringified IP addresses.
    #[serde(default)]
    pub dns: Vec<IpAddr>,
}

#[derive(Clone, Deserialize)]
pub struct Routes {
    /// Ipv4 gateway.
    pub gateway4: Option<Gateway>,

    /// Ipv6 gateway.
    pub gateway6: Option<Gateway>,
}

#[derive(Clone, Deserialize)]
pub struct Gateway {
    /// Outgoing network device.
    pub dev: String,

    /// Stringified gateway IP address.
    pub gateway: IpAddr,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum InterfaceState {
    Up,
    Down,
    #[serde(other)]
    Unknown,
}

impl InterfaceState {
    // avoid display trait as this is not the string representation for a serializer
    pub fn render(&self) -> String {
        match self {
            Self::Up => "\u{25CF}",
            Self::Down | Self::Unknown => " ",
        }
        .into()
    }
}

#[derive(Clone, Deserialize)]
pub struct Interface {
    pub name: String,

    pub index: usize,

    pub mac: String,

    pub state: InterfaceState,

    #[serde(default)]
    #[serde(deserialize_with = "deserialize_cidr_list")]
    pub addresses: Option<Vec<CidrAddress>>,
}

impl Interface {
    // avoid display trait as this is not the string representation for a serializer
    pub fn render(&self) -> String {
        format!("{} {}", self.state.render(), self.name)
    }
}

pub fn spawn_low_level_installer(test_mode: bool) -> io::Result<process::Child> {
    let (path, args, envs): (&str, &[&str], Vec<(&str, &str)>) = if test_mode {
        (
            "./proxmox-low-level-installer",
            &["-t", "/dev/null", "start-session-test"],
            vec![("PERL5LIB", ".")],
        )
    } else {
        ("proxmox-low-level-installer", &["start-session"], vec![])
    };

    Command::new(path)
        .args(args)
        .envs(envs)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
}

/// See Proxmox::Install::Config
#[derive(Debug, Deserialize, Serialize)]
pub struct InstallConfig {
    pub autoreboot: usize,

    #[serde(
        serialize_with = "serialize_fstype",
        deserialize_with = "deserialize_fs_type"
    )]
    pub filesys: FsType,
    pub hdsize: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swapsize: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maxroot: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minfree: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maxvz: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub zfs_opts: Option<InstallZfsOption>,

    #[serde(
        serialize_with = "serialize_disk_opt",
        skip_serializing_if = "Option::is_none",
        // only the 'path' property is serialized -> deserialization is problematic
        // The information would be present in the 'run-env-info-json', but for now there is no
        // need for it in any code that deserializes the low-level config. Therefore we are
        // currently skipping it on deserialization
        skip_deserializing
    )]
    pub target_hd: Option<Disk>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub disk_selection: BTreeMap<String, String>,

    pub existing_storage_auto_rename: usize,

    pub country: String,
    pub timezone: String,
    pub keymap: String,

    pub password: String,
    pub mailto: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub root_ssh_keys: Vec<String>,

    pub mngmt_nic: String,

    pub hostname: String,
    pub domain: String,
    #[serde(serialize_with = "serialize_as_display")]
    pub cidr: CidrAddress,
    pub gateway: IpAddr,
    pub dns: IpAddr,
}

fn serialize_disk_opt<S>(value: &Option<Disk>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(disk) = value {
        serializer.serialize_str(&disk.path)
    } else {
        serializer.serialize_none()
    }
}

fn serialize_fstype<S>(value: &FsType, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    use FsType::*;
    let value = match value {
        // proxinstall::$fssetup
        Ext4 => "ext4",
        Xfs => "xfs",
        // proxinstall::get_zfs_raid_setup()
        Zfs(ZfsRaidLevel::Raid0) => "zfs (RAID0)",
        Zfs(ZfsRaidLevel::Raid1) => "zfs (RAID1)",
        Zfs(ZfsRaidLevel::Raid10) => "zfs (RAID10)",
        Zfs(ZfsRaidLevel::RaidZ) => "zfs (RAIDZ-1)",
        Zfs(ZfsRaidLevel::RaidZ2) => "zfs (RAIDZ-2)",
        Zfs(ZfsRaidLevel::RaidZ3) => "zfs (RAIDZ-3)",
        // proxinstall::get_btrfs_raid_setup()
        Btrfs(BtrfsRaidLevel::Raid0) => "btrfs (RAID0)",
        Btrfs(BtrfsRaidLevel::Raid1) => "btrfs (RAID1)",
        Btrfs(BtrfsRaidLevel::Raid10) => "btrfs (RAID10)",
    };

    serializer.collect_str(value)
}

pub fn deserialize_fs_type<'de, D>(deserializer: D) -> Result<FsType, D::Error>
where
    D: Deserializer<'de>,
{
    use FsType::*;
    let de_fs: String = Deserialize::deserialize(deserializer)?;

    match de_fs.as_str() {
        "ext4" => Ok(Ext4),
        "xfs" => Ok(Xfs),
        "zfs (RAID0)" => Ok(Zfs(ZfsRaidLevel::Raid0)),
        "zfs (RAID1)" => Ok(Zfs(ZfsRaidLevel::Raid1)),
        "zfs (RAID10)" => Ok(Zfs(ZfsRaidLevel::Raid10)),
        "zfs (RAIDZ-1)" => Ok(Zfs(ZfsRaidLevel::RaidZ)),
        "zfs (RAIDZ-2)" => Ok(Zfs(ZfsRaidLevel::RaidZ2)),
        "zfs (RAIDZ-3)" => Ok(Zfs(ZfsRaidLevel::RaidZ3)),
        "btrfs (RAID0)" => Ok(Btrfs(BtrfsRaidLevel::Raid0)),
        "btrfs (RAID1)" => Ok(Btrfs(BtrfsRaidLevel::Raid1)),
        "btrfs (RAID10)" => Ok(Btrfs(BtrfsRaidLevel::Raid10)),
        _ => Err(de::Error::custom("could not find file system: {de_fs}")),
    }
}
