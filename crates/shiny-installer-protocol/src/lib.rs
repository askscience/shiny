//! Shared contract between the PEAK'D! installer plugin and the privileged helper.
//!
//! The plugin runs as the live user; the helper runs as root and owns every
//! destructive operation. Keeping the types here means neither side can drift
//! from the other without a compile error.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 4;

pub mod methods {
    pub const HEALTH: &str = "health";
    pub const OPTIONS: &str = "options";
    pub const PROBE: &str = "probe";
    pub const VALIDATE: &str = "validate";
    pub const APPLY: &str = "apply";
    pub const STATUS: &str = "status";
    pub const CANCEL: &str = "cancel";
}

pub const PROFILE_BACKPORTS: &str = "backports";
pub const PROFILE_T2: &str = "t2";

pub const MODE_ERASE: &str = "erase";
pub const MODE_EXISTING: &str = "existing";
pub const MODE_MANUAL: &str = "manual";
pub const FS_EXT4: &str = "ext4";
pub const FS_VFAT: &str = "vfat";
pub const FS_SWAP: &str = "swap";
pub const ENCRYPTION_NONE: &str = "none";
pub const BOOTLOADER_GRUB: &str = "grub";

pub const NVIDIA_DRIVER_LICENSE_ID: &str = "nvidia-driver-2025-02-25";
pub const NVIDIA_DRIVER_LICENSE_VERSION: &str = "2025-02-25";
pub const NVIDIA_DRIVER_LICENSE_URL: &str =
    "https://www.nvidia.com/en-us/drivers/nvidia-license/linux/";
pub const NVIDIA_REDIST_LICENSE_URL: &str =
    "https://developer.download.nvidia.com/compute/nvidia-driver/redist/nvidia_driver/LICENSE.txt";
pub const CUDA_LICENSE_ID: &str = "cuda-toolkit-eula";
pub const CUDA_LICENSE_VERSION: &str = "current";
pub const CUDA_LICENSE_URL: &str = "https://docs.nvidia.com/cuda/eula/index.html";
pub const NVIDIA_DEBIAN_COPYRIGHT_URL: &str =
    "https://launchpad.net/debian/sid/+source/nvidia-graphics-drivers/+copyright";

pub const AI_TIER_NONE: &str = "none";
pub const AI_TIER_VOICE: &str = "voice";
pub const AI_TIER_LOCAL: &str = "local";

const MIN_ROOT_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const ESP_MIN_BYTES: u64 = 512 * 1024 * 1024;
const ESP_MAX_BYTES: u64 = 1024 * 1024 * 1024;

/// DMI/T2/GPU facts the helper reads on the target machine.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HardwareInfo {
    pub vendor: String,
    pub product: String,
    pub board: String,
    pub is_t2: bool,
    pub firmware: String,
    pub secure_boot: bool,
    pub gpus: Vec<GpuInfo>,
    /// Device backing `/run/live/medium`, when it can be resolved.
    pub live_medium: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GpuInfo {
    pub vendor: String,
    pub model: String,
    pub pci_id: String,
    pub is_nvidia: bool,
    pub is_hybrid: bool,
    pub is_datacenter: bool,
}

/// One `lsblk` node. Whole disks are returned in `ProbeResponse::disks`;
/// partitions stay in `children` so the UI can show mounted/foreign layouts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DiskInfo {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub size_bytes: u64,
    /// Absolute byte offset on the disk. `0` means unknown or approximate.
    pub start_bytes: u64,
    pub model: String,
    pub serial: String,
    pub tran: String,
    pub removable: bool,
    pub read_only: bool,
    pub is_loop: bool,
    pub is_rom: bool,
    pub mountpoints: Vec<String>,
    pub fstype: String,
    pub pttype: String,
    pub partuuid: String,
    pub uuid: String,
    pub label: String,
    pub fsver: String,
    pub parttypename: String,
    pub partflags: Vec<String>,
    pub holders: Vec<String>,
    pub free_extents: Vec<FreeExtent>,
    pub children: Vec<DiskInfo>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FreeExtent {
    pub start_bytes: u64,
    pub size_bytes: u64,
}

impl DiskInfo {
    /// True when this node or any descendant is mounted.
    pub fn has_mounts(&self) -> bool {
        !self.mountpoints.is_empty() || self.children.iter().any(DiskInfo::has_mounts)
    }

    /// Every device path in this node and its descendants.
    pub fn paths(&self) -> Vec<String> {
        let mut out = vec![self.path.clone()];
        for child in &self.children {
            out.extend(child.paths());
        }
        out
    }
}

/// The disk the user selected. The helper re-checks it against a fresh probe.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TargetDisk {
    pub path: String,
    pub serial: String,
    pub model: String,
    pub size_bytes: u64,
}

/// Stable identity for an existing partition selected by the user.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PartitionRef {
    pub disk_path: String,
    pub disk_serial: String,
    pub path: String,
    pub partuuid: String,
    pub uuid: String,
    pub size_bytes: u64,
    pub fstype: String,
}

/// One planned partition, used by erase and manual free-space layouts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PartitionSpec {
    pub role: String,
    pub size_bytes: Option<u64>,
    pub size_percent: Option<u8>,
    pub fs: String,
    pub mount: Option<String>,
    pub label: Option<String>,
    pub flags: Vec<String>,
    pub wipe: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PartitionLayout {
    pub table: String,
    /// Manual mode: the free extent chosen by the user. The engine creates the
    /// new partitions from `start_bytes`; it is never re-picked automatically.
    pub free_extent: Option<FreeExtent>,
    pub partitions: Vec<PartitionSpec>,
    pub preserve: Vec<PartitionRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct NvidiaPlan {
    pub install_driver: bool,
    pub install_cuda: bool,
    pub accept_driver_license: bool,
    pub accept_cuda_license: bool,
    pub secure_boot_mok: bool,
}

/// A license the user explicitly accepted in the wizard. No secrets belong here.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LicenseAcceptance {
    pub id: String,
    pub version: String,
    pub hash: String,
    pub accepted_at: String,
}

/// Everything the installer needs. The helper validates it and re-probes the
/// disk before applying anything; the plugin never sends shell commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InstallPlan {
    pub target: TargetDisk,
    pub target_partition: Option<PartitionRef>,
    pub esp_partition: Option<PartitionRef>,
    pub layout: PartitionLayout,
    pub bootloader: String,
    pub mode: String,
    pub filesystem: String,
    pub encryption: String,
    pub profile: String,
    pub locale: String,
    pub keyboard: String,
    pub timezone: String,
    pub full_name: String,
    pub username: String,
    pub password: String,
    pub hostname: String,
    pub install_voice: bool,
    pub ai_tier: String,
    pub nvidia: NvidiaPlan,
    pub licenses: Vec<LicenseAcceptance>,
}

impl Default for InstallPlan {
    fn default() -> Self {
        Self {
            target: TargetDisk::default(),
            target_partition: None,
            esp_partition: None,
            layout: PartitionLayout {
                table: "gpt".into(),
                ..PartitionLayout::default()
            },
            bootloader: BOOTLOADER_GRUB.into(),
            mode: MODE_ERASE.into(),
            filesystem: FS_EXT4.into(),
            encryption: ENCRYPTION_NONE.into(),
            profile: PROFILE_BACKPORTS.into(),
            locale: "en_US.UTF-8".into(),
            keyboard: "us".into(),
            timezone: "UTC".into(),
            full_name: String::new(),
            username: String::new(),
            password: String::new(),
            hostname: String::new(),
            install_voice: true,
            ai_tier: AI_TIER_VOICE.into(),
            nvidia: NvidiaPlan::default(),
            licenses: Vec::new(),
        }
    }
}

impl InstallPlan {
    pub fn is_t2(&self) -> bool {
        self.profile == PROFILE_T2
    }

    pub fn has_license(&self, id: &str) -> bool {
        self.licenses
            .iter()
            .any(|license| license.id == id && !license.version.trim().is_empty())
    }

    /// Device-independent validation. The helper adds disk/hardware checks.
    pub fn basic_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if !self.target.path.starts_with("/dev/") {
            errors.push("target disk path must be an absolute /dev/ path".into());
        }
        if self.target.size_bytes == 0 {
            errors.push("target disk size is missing".into());
        }
        if self.target.serial.trim().is_empty() {
            errors.push("target disk serial is required for destructive install".into());
        }
        if !matches!(self.mode.as_str(), MODE_ERASE | MODE_EXISTING | MODE_MANUAL) {
            errors.push(format!("unsupported install mode: {}", self.mode));
        }
        if self.bootloader != BOOTLOADER_GRUB {
            errors.push(format!("unsupported bootloader: {}", self.bootloader));
        }
        if self.filesystem != FS_EXT4 {
            errors.push(format!("unsupported filesystem: {}", self.filesystem));
        }
        if self.encryption != ENCRYPTION_NONE {
            errors.push(format!(
                "unsupported encryption in this build: {}",
                self.encryption
            ));
        }
        if self.profile != PROFILE_BACKPORTS && self.profile != PROFILE_T2 {
            errors.push(format!("unknown kernel profile: {}", self.profile));
        }

        match self.mode.as_str() {
            MODE_ERASE => {
                if self.target_partition.is_some() || self.esp_partition.is_some() {
                    errors.push("erase mode cannot reference existing partitions".into());
                }
                if !self.layout.preserve.is_empty() {
                    errors.push("erase mode cannot preserve existing partitions".into());
                }
                if self.layout.partitions.is_empty() {
                    errors.push("erase mode requires a partition layout".into());
                }
                if self.layout.free_extent.is_some() {
                    errors.push("erase mode cannot target a free extent".into());
                }
                errors.extend(self.layout_errors(self.target.size_bytes));
            }
            MODE_EXISTING => {
                if self.target_partition.is_none() {
                    errors.push("existing-partition mode requires a root partition".into());
                }
                if self.esp_partition.is_none() {
                    errors.push("existing-partition mode requires an EFI system partition".into());
                }
                if !self.layout.partitions.is_empty() {
                    errors.push(
                        "existing-partition mode cannot create partitions; use manual mode".into(),
                    );
                }
                if let Some(root) = &self.target_partition {
                    errors.extend(self.partition_ref_errors(root, "root"));
                    if root.fstype != FS_EXT4 && !root.fstype.is_empty() {
                        errors.push(format!(
                            "existing root partition must be ext4 or blank, got {}",
                            root.fstype
                        ));
                    }
                }
                if let Some(esp) = &self.esp_partition {
                    errors.extend(self.partition_ref_errors(esp, "EFI system"));
                    if esp.fstype != FS_VFAT {
                        errors.push(format!(
                            "EFI system partition must be vfat, got {}",
                            esp.fstype
                        ));
                    }
                }
                if let (Some(root), Some(esp)) = (&self.target_partition, &self.esp_partition) {
                    if root.path == esp.path {
                        errors.push("root and EFI system partitions must differ".into());
                    }
                }
            }
            MODE_MANUAL => {
                if self.target_partition.is_some() || self.esp_partition.is_some() {
                    errors.push("manual mode cannot reference a single target partition".into());
                }
                if self.layout.partitions.is_empty() {
                    errors.push("manual mode requires a partition layout".into());
                }
                match self.layout.free_extent.as_ref() {
                    Some(extent) if extent.size_bytes == 0 => {
                        errors.push("the chosen free extent is empty".into());
                        errors.extend(self.layout_errors(0));
                    }
                    Some(extent) => {
                        if extent.start_bytes.saturating_add(extent.size_bytes)
                            > self.target.size_bytes
                        {
                            errors.push("the chosen free extent is outside the target disk".into());
                        }
                        errors.extend(self.layout_errors(extent.size_bytes));
                    }
                    None => {
                        errors.push("manual mode requires the chosen free space".into());
                        errors.extend(self.layout_errors(self.target.size_bytes));
                    }
                }
            }
            _ => {}
        }

        if self.nvidia.install_cuda && !self.nvidia.install_driver {
            errors.push("CUDA requires the NVIDIA driver".into());
        }
        if self.nvidia.install_driver && !self.nvidia.accept_driver_license {
            errors.push("the NVIDIA driver license must be accepted".into());
        }
        if self.nvidia.install_cuda && !self.nvidia.accept_cuda_license {
            errors.push("the CUDA license must be accepted".into());
        }
        if self.nvidia.install_driver && !self.has_license(NVIDIA_DRIVER_LICENSE_ID) {
            errors.push("the accepted NVIDIA driver license record is missing".into());
        }
        if self.nvidia.install_cuda && !self.has_license(CUDA_LICENSE_ID) {
            errors.push("the accepted CUDA license record is missing".into());
        }

        for (name, value) in [
            ("locale", &self.locale),
            ("keyboard", &self.keyboard),
            ("timezone", &self.timezone),
            ("full name", &self.full_name),
            ("username", &self.username),
            ("hostname", &self.hostname),
        ] {
            if value.trim().is_empty() {
                errors.push(format!("{name} is required"));
            }
        }

        if self.password.is_empty() {
            errors.push("an account password is required".into());
        } else if self.password.len() < 8 {
            errors.push("the account password must be at least 8 characters".into());
        }

        if !self.username.is_empty() && !valid_username(&self.username) {
            errors.push(
                "username must start with a lowercase letter or '_' and contain only \
                 lowercase letters, digits, '_' or '-' (max 32)"
                    .into(),
            );
        }
        if self.username == "root" {
            errors.push("the first account cannot be root".into());
        }
        if !self.hostname.is_empty() && !valid_hostname(&self.hostname) {
            errors.push(
                "hostname must be 1–63 chars, start/end alphanumeric, and use only \
                 letters, digits, '-' or '.'"
                    .into(),
            );
        }
        if !matches!(
            self.ai_tier.as_str(),
            AI_TIER_NONE | AI_TIER_VOICE | AI_TIER_LOCAL
        ) {
            errors.push(format!("unknown AI tier: {}", self.ai_tier));
        }
        if self.install_voice && self.ai_tier == AI_TIER_NONE {
            errors.push("install_voice requires the voice AI tier".into());
        }

        errors
    }

    fn partition_ref_errors(&self, partition: &PartitionRef, label: &str) -> Vec<String> {
        let mut errors = Vec::new();
        if !partition.path.starts_with("/dev/") {
            errors.push(format!("{label} path must be an absolute /dev/ path"));
        }
        if partition.disk_path != self.target.path {
            errors.push(format!("{label} belongs to a different disk"));
        }
        if !partition.disk_serial.is_empty() && partition.disk_serial != self.target.serial {
            errors.push(format!("{label} disk serial does not match the target"));
        }
        if partition.size_bytes == 0 {
            errors.push(format!("{label} size is missing"));
        }
        if partition.partuuid.trim().is_empty() && partition.uuid.trim().is_empty() {
            errors.push(format!("{label} has no stable PARTUUID or UUID"));
        }
        errors
    }

    fn layout_errors(&self, limit_bytes: u64) -> Vec<String> {
        let mut errors = Vec::new();
        if self.layout.table != "gpt" {
            errors.push(format!(
                "unsupported partition table: {}",
                self.layout.table
            ));
        }

        let mut roots = 0;
        let mut esps = 0;
        let mut homes = 0;
        let mut swaps = 0;
        let mut fixed_bytes = 0u64;
        let mut percent_total = 0u16;

        for partition in &self.layout.partitions {
            if partition.size_bytes.is_some() && partition.size_percent.is_some() {
                errors.push(format!(
                    "{} partition cannot set both size and percent",
                    partition.role
                ));
            }
            if let Some(percent) = partition.size_percent {
                if percent == 0 || percent > 100 {
                    errors.push(format!(
                        "{} partition percent must be 1–100",
                        partition.role
                    ));
                } else {
                    percent_total += u16::from(percent);
                }
            }
            if let Some(size) = partition.size_bytes {
                fixed_bytes = fixed_bytes.saturating_add(size);
            }

            match partition.role.as_str() {
                "root" => {
                    roots += 1;
                    if partition.fs != FS_EXT4 {
                        errors.push("root partition must use ext4".into());
                    }
                    if partition.mount.as_deref() != Some("/") {
                        errors.push("root partition must be mounted at /".into());
                    }
                    if let Some(size) = partition.size_bytes {
                        if size < MIN_ROOT_BYTES {
                            errors.push(format!(
                                "root partition must be at least {} GiB",
                                MIN_ROOT_BYTES / 1024 / 1024 / 1024
                            ));
                        }
                    }
                }
                "esp" => {
                    esps += 1;
                    if partition.fs != FS_VFAT {
                        errors.push("EFI system partition must use vfat".into());
                    }
                    if partition.mount.as_deref() != Some("/boot/efi") {
                        errors.push("EFI system partition must be mounted at /boot/efi".into());
                    }
                    if let Some(size) = partition.size_bytes {
                        if !(ESP_MIN_BYTES..=ESP_MAX_BYTES).contains(&size) {
                            errors.push("EFI system partition must be 512 MiB–1 GiB".into());
                        }
                    } else {
                        errors.push("EFI system partition needs an explicit size".into());
                    }
                }
                "home" => {
                    homes += 1;
                    if partition.fs != FS_EXT4 {
                        errors.push("home partition must use ext4".into());
                    }
                    if partition.mount.as_deref() != Some("/home") {
                        errors.push("home partition must be mounted at /home".into());
                    }
                }
                "swap" => {
                    swaps += 1;
                    if partition.fs != FS_SWAP {
                        errors.push("swap partition must use swap".into());
                    }
                }
                other => errors.push(format!("unknown partition role: {other}")),
            }
        }

        if roots != 1 {
            errors.push("the layout must contain exactly one root partition".into());
        }
        if esps != 1 {
            errors.push("the layout must contain exactly one EFI system partition".into());
        }
        if homes > 1 {
            errors.push("the layout can contain at most one home partition".into());
        }
        if swaps > 1 {
            errors.push("the layout can contain at most one swap partition".into());
        }
        if fixed_bytes > limit_bytes {
            errors.push("the planned partitions exceed the available space".into());
        }
        if percent_total > 100 {
            errors.push("partition percentages exceed 100%".into());
        }

        for partition in &self.layout.preserve {
            if !partition.path.starts_with("/dev/") {
                errors.push("preserved partition path must be an absolute /dev/ path".into());
            }
            if partition.disk_path != self.target.path {
                errors.push("preserved partition belongs to a different disk".into());
            }
        }

        errors
    }
}

fn valid_username(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if value.len() > 32 {
        return false;
    }
    if !(first.is_ascii_lowercase() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn valid_hostname(value: &str) -> bool {
    if value.is_empty() || value.len() > 63 {
        return false;
    }
    if value.starts_with('-') || value.ends_with('-') {
        return false;
    }
    value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeResponse {
    pub disks: Vec<DiskInfo>,
    pub hardware: HardwareInfo,
    pub live_medium: Option<String>,
}

/// Locale, keyboard and timezone data read from the live system.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemOptions {
    pub locales: Vec<String>,
    pub keyboards: Vec<String>,
    pub timezones: Vec<String>,
    pub default_locale: String,
    pub default_keyboard: String,
    pub default_timezone: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ValidationResult {
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub hardware: HardwareInfo,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JobStatus {
    pub job: String,
    pub stage: String,
    pub progress: u8,
    pub done: bool,
    pub success: bool,
    pub error: Option<String>,
    pub log: Vec<String>,
}

/// The apply request carries the typed plan plus the serial the user typed in
/// the wizard. The helper compares it with the plan target before doing work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyRequest {
    pub plan: InstallPlan,
    #[serde(default)]
    pub confirm_serial: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Health {
    pub version: String,
    pub protocol: u32,
    pub dry_run: bool,
    pub root: bool,
    pub socket: String,
    pub hardware: HardwareInfo,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: PROTOCOL_VERSION,
            dry_run: true,
            root: false,
            socket: String::new(),
            hardware: HardwareInfo::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl RpcRequest {
    pub fn new(id: impl Into<String>, method: impl Into<String>, params: Value) -> Self {
        Self {
            id: id.into(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub id: String,
    pub ok: bool,
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

impl RpcResponse {
    pub fn ok(id: impl Into<String>, data: Value) -> Self {
        Self {
            id: id.into(),
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ok: false,
            data: None,
            error: Some(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_layout() -> PartitionLayout {
        PartitionLayout {
            table: "gpt".into(),
            partitions: vec![
                PartitionSpec {
                    role: "esp".into(),
                    size_bytes: Some(512 * 1024 * 1024),
                    fs: FS_VFAT.into(),
                    mount: Some("/boot/efi".into()),
                    flags: vec!["esp".into()],
                    wipe: true,
                    ..PartitionSpec::default()
                },
                PartitionSpec {
                    role: "root".into(),
                    size_bytes: Some(20 * 1024 * 1024 * 1024),
                    fs: FS_EXT4.into(),
                    mount: Some("/".into()),
                    wipe: true,
                    ..PartitionSpec::default()
                },
            ],
            ..PartitionLayout::default()
        }
    }

    fn valid_plan() -> InstallPlan {
        InstallPlan {
            target: TargetDisk {
                path: "/dev/nvme0n1".into(),
                serial: "S1".into(),
                model: "Disk".into(),
                size_bytes: 100 * 1024 * 1024 * 1024,
            },
            layout: default_layout(),
            full_name: "Ada Lovelace".into(),
            username: "ada".into(),
            password: "correct-horse".into(),
            hostname: "ada-peakd".into(),
            ..InstallPlan::default()
        }
    }

    #[test]
    fn valid_plan_has_no_errors() {
        assert!(valid_plan().basic_errors().is_empty());
    }

    #[test]
    fn rejects_bad_username_and_profile() {
        let mut plan = valid_plan();
        plan.username = "Root".into();
        plan.profile = "t3".into();
        let errors = plan.basic_errors();
        assert!(errors.iter().any(|e| e.contains("username")));
        assert!(errors.iter().any(|e| e.contains("kernel profile")));
    }

    #[test]
    fn serial_is_required() {
        let mut plan = valid_plan();
        plan.target.serial.clear();
        assert!(plan
            .basic_errors()
            .iter()
            .any(|error| error.contains("serial")));
    }

    fn valid_manual_plan() -> InstallPlan {
        let mut plan = valid_plan();
        plan.mode = MODE_MANUAL.into();
        plan.layout.free_extent = Some(FreeExtent {
            start_bytes: 40 * 1024 * 1024 * 1024,
            size_bytes: 40 * 1024 * 1024 * 1024,
        });
        plan
    }

    #[test]
    fn valid_manual_plan_has_no_errors() {
        assert!(valid_manual_plan().basic_errors().is_empty());
    }

    #[test]
    fn manual_plan_needs_a_chosen_free_extent() {
        let mut plan = valid_manual_plan();
        plan.layout.free_extent = None;
        assert!(plan
            .basic_errors()
            .iter()
            .any(|error| error.contains("chosen free space")));
    }

    #[test]
    fn manual_plan_rejects_an_extent_outside_the_disk() {
        let mut plan = valid_manual_plan();
        plan.layout.free_extent = Some(FreeExtent {
            start_bytes: 95 * 1024 * 1024 * 1024,
            size_bytes: 10 * 1024 * 1024 * 1024,
        });
        assert!(plan
            .basic_errors()
            .iter()
            .any(|error| error.contains("outside the target disk")));
    }

    #[test]
    fn manual_plan_rejects_an_extent_that_is_too_small() {
        let mut plan = valid_manual_plan();
        plan.layout.free_extent = Some(FreeExtent {
            start_bytes: 40 * 1024 * 1024 * 1024,
            size_bytes: 10 * 1024 * 1024 * 1024,
        });
        assert!(plan
            .basic_errors()
            .iter()
            .any(|error| error.contains("available space")));
    }

    #[test]
    fn erase_plan_rejects_a_free_extent() {
        let mut plan = valid_plan();
        plan.layout.free_extent = Some(FreeExtent {
            start_bytes: 0,
            size_bytes: 40 * 1024 * 1024 * 1024,
        });
        assert!(plan
            .basic_errors()
            .iter()
            .any(|error| error.contains("erase mode cannot target a free extent")));
    }

    #[test]
    fn existing_partition_mode_needs_root_and_esp() {
        let mut plan = valid_plan();
        plan.mode = MODE_EXISTING.into();
        plan.layout.partitions.clear();
        plan.target_partition = Some(PartitionRef {
            disk_path: "/dev/nvme0n1".into(),
            disk_serial: "S1".into(),
            path: "/dev/nvme0n1p2".into(),
            partuuid: "part-root".into(),
            uuid: "fs-root".into(),
            size_bytes: 60 * 1024 * 1024 * 1024,
            fstype: FS_EXT4.into(),
        });
        plan.esp_partition = Some(PartitionRef {
            disk_path: "/dev/nvme0n1".into(),
            disk_serial: "S1".into(),
            path: "/dev/nvme0n1p1".into(),
            partuuid: "part-esp".into(),
            uuid: "fs-esp".into(),
            size_bytes: 512 * 1024 * 1024,
            fstype: FS_VFAT.into(),
        });
        assert!(plan.basic_errors().is_empty());
        plan.target_partition = None;
        assert!(plan
            .basic_errors()
            .iter()
            .any(|error| error.contains("root partition")));
    }

    #[test]
    fn nvidia_license_is_required() {
        let mut plan = valid_plan();
        plan.nvidia.install_driver = true;
        assert!(plan
            .basic_errors()
            .iter()
            .any(|error| error.contains("NVIDIA driver license")));
        plan.nvidia.accept_driver_license = true;
        plan.licenses.push(LicenseAcceptance {
            id: NVIDIA_DRIVER_LICENSE_ID.into(),
            version: NVIDIA_DRIVER_LICENSE_VERSION.into(),
            hash: String::new(),
            accepted_at: "2026-10-23T00:00:00Z".into(),
        });
        assert!(!plan
            .basic_errors()
            .iter()
            .any(|error| error.contains("NVIDIA driver license")));
    }

    #[test]
    fn rpc_response_round_trips() {
        let response = RpcResponse::ok("1", serde_json::json!({ "hello": "world" }));
        let encoded = serde_json::to_string(&response).unwrap();
        let decoded: RpcResponse = serde_json::from_str(&encoded).unwrap();
        assert!(decoded.ok);
        assert_eq!(decoded.data.unwrap()["hello"], "world");
    }
}
