//! Read-only disk/hardware probing and plan validation.
//!
//! The helper never trusts the plan: it re-probes the machine, matches the
//! target by path/serial/model/size, and refuses mounted, removable, read-only,
//! loop, optical or live-medium devices.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use shiny_installer_protocol::{
    DiskInfo, FreeExtent, GpuInfo, HardwareInfo, InstallPlan, PartitionRef, ProbeResponse,
    SystemOptions, ValidationResult, MODE_ERASE, MODE_EXISTING, MODE_MANUAL,
};

const MIN_DISK_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const WARN_DISK_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const MIN_ROOT_BYTES: u64 = 20 * 1024 * 1024 * 1024;
/// Headroom for the MiB rounding the engine applies to the chosen extent.
const PLACEMENT_GUARD_BYTES: u64 = 1024 * 1024;

pub fn hardware() -> HardwareInfo {
    let vendor = read_trim("/sys/class/dmi/id/sys_vendor");
    let product = read_trim("/sys/class/dmi/id/product_name");
    let board = read_trim("/sys/class/dmi/id/board_name");
    let live_medium = live_medium_device();
    let gpus = detect_gpus();
    HardwareInfo {
        vendor,
        product,
        board,
        is_t2: detects_t2(),
        firmware: if Path::new("/sys/firmware/efi").exists() {
            "uefi".into()
        } else {
            "bios".into()
        },
        secure_boot: secure_boot_enabled(),
        gpus,
        live_medium,
    }
}

pub fn probe() -> Result<ProbeResponse, String> {
    let output = Command::new("lsblk")
        .args([
            "-J",
            "-b",
            "-o",
            "NAME,PATH,TYPE,SIZE,MODEL,SERIAL,TRAN,RM,RO,MOUNTPOINTS,FSTYPE,PTTYPE,PARTUUID,UUID,LABEL,FSVER,PARTTYPENAME,PARTFLAGS",
        ])
        .output()
        .map_err(|e| format!("cannot run lsblk: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "lsblk failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let value: Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("invalid lsblk JSON: {e}"))?;
    let mut disks = Vec::new();
    if let Some(devices) = value.get("blockdevices").and_then(Value::as_array) {
        for node in devices {
            let mut disk = parse_disk(node);
            if disk.kind == "disk" && !disk.is_loop && !disk.is_rom {
                let layout = read_disk_layout(&disk.path);
                disk.free_extents = layout.free;
                let disk_name = disk.name.clone();
                for child in &mut disk.children {
                    if let Some(number) = partition_number(&child.name, &disk_name) {
                        if let Some(start) = layout.starts.get(&number) {
                            child.start_bytes = *start;
                        }
                    }
                }
                disks.push(disk);
            }
        }
    }

    let hardware = hardware();
    Ok(ProbeResponse {
        disks,
        live_medium: hardware.live_medium.clone(),
        hardware,
    })
}

pub fn validate(plan: &InstallPlan) -> Result<ValidationResult, String> {
    let probe = probe()?;
    let mut errors = plan.basic_errors();
    let mut warnings = Vec::new();

    let target = probe
        .disks
        .iter()
        .find(|disk| disk.path == plan.target.path)
        .cloned();

    let Some(target) = target else {
        errors.push(format!(
            "target disk {} was not found in a fresh probe",
            plan.target.path
        ));
        return Ok(ValidationResult {
            ok: false,
            errors,
            warnings,
            hardware: probe.hardware,
        });
    };

    if target.kind != "disk" {
        errors.push("target must be a whole disk, not a partition".into());
    }
    if target.removable {
        errors.push("target disk is removable; the live medium and USB disks are refused".into());
    }
    if target.read_only {
        errors.push("target disk is read-only".into());
    }
    if target.is_loop || target.is_rom {
        errors.push("loop and optical devices cannot be install targets".into());
    }
    if probe.hardware.firmware != "uefi" {
        errors.push("UEFI firmware is required for this installer build".into());
    }
    if let Some(live) = probe.live_medium.as_deref() {
        if target.paths().iter().any(|path| path == live) {
            errors.push("target disk is the live medium".into());
        }
    }
    if !matches!(target.pttype.as_str(), "" | "gpt") {
        errors.push(format!(
            "only GPT targets are supported in this build, found {}",
            target.pttype
        ));
    }

    if !plan.target.serial.is_empty() && plan.target.serial != target.serial {
        errors.push(format!(
            "target serial changed: plan={} probe={}",
            plan.target.serial, target.serial
        ));
    }
    if !plan.target.model.is_empty() && plan.target.model != target.model {
        errors.push(format!(
            "target model changed: plan={} probe={}",
            plan.target.model, target.model
        ));
    }
    if plan.target.size_bytes != 0 && plan.target.size_bytes != target.size_bytes {
        errors.push(format!(
            "target size changed: plan={} probe={}",
            plan.target.size_bytes, target.size_bytes
        ));
    }

    if target.size_bytes < MIN_DISK_BYTES {
        errors.push(format!(
            "target disk is too small: {} GiB (minimum {} GiB)",
            target.size_bytes / 1024 / 1024 / 1024,
            MIN_DISK_BYTES / 1024 / 1024 / 1024
        ));
    } else if target.size_bytes < WARN_DISK_BYTES {
        warnings.push(format!(
            "small target disk: {} GiB; the voice pack may not fit comfortably",
            target.size_bytes / 1024 / 1024 / 1024
        ));
    }

    match plan.mode.as_str() {
        MODE_ERASE => validate_erase(plan, &target, &mut errors, &mut warnings),
        MODE_EXISTING => validate_existing(plan, &target, &mut errors, &mut warnings),
        MODE_MANUAL => validate_manual(plan, &target, &mut errors, &mut warnings),
        _ => {}
    }

    validate_nvidia(plan, &probe.hardware, &mut errors, &mut warnings);

    if plan.is_t2() && !probe.hardware.is_t2 {
        errors.push("the T2 profile was requested on a non-T2 machine".into());
    }
    if !plan.is_t2() && probe.hardware.is_t2 {
        warnings.push(
            "T2 hardware detected: the generic backports profile may not enable keyboard, \
             trackpad, Wi-Fi, Bluetooth or audio"
                .into(),
        );
    }
    if plan.profile == shiny_installer_protocol::PROFILE_T2 && probe.hardware.product.is_empty() {
        warnings.push("T2 profile selected but the DMI product name is unavailable".into());
    }

    Ok(ValidationResult {
        ok: errors.is_empty(),
        errors,
        warnings,
        hardware: probe.hardware,
    })
}

fn validate_erase(
    plan: &InstallPlan,
    target: &DiskInfo,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    if target.has_mounts() {
        errors.push("target disk or one of its partitions is mounted; unmount it first".into());
    }
    if target.children.iter().any(is_apfs) {
        errors.push(
            "the target contains APFS/HFS partitions; this build never erases an Apple layout"
                .into(),
        );
    }
    if target
        .children
        .iter()
        .any(|child| !child.holders.is_empty())
    {
        errors.push("the target has active device-mapper holders; close them first".into());
    }
    if plan
        .layout
        .preserve
        .iter()
        .any(|partition| !partition.path.is_empty())
    {
        errors.push("erase mode cannot preserve partitions".into());
    }
    if target.children.is_empty() {
        warnings
            .push("the target has no existing partition table; the engine will create GPT".into());
    }
}

fn validate_existing(
    plan: &InstallPlan,
    target: &DiskInfo,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let Some(root_ref) = plan.target_partition.as_ref() else {
        return;
    };
    let Some(root) = find_partition(target, root_ref) else {
        errors.push(format!(
            "root partition {} was not found on {}",
            root_ref.path, target.path
        ));
        return;
    };

    if !root.mountpoints.is_empty() {
        errors.push("the selected root partition is mounted; unmount it first".into());
    }
    if !root.holders.is_empty() {
        errors.push("the selected root partition has active holders".into());
    }
    if is_apfs(root) {
        errors.push("an APFS/HFS partition cannot be the root target".into());
    }
    if is_foreign_root(root) {
        errors.push(format!(
            "partition {} has an unsupported filesystem or role",
            root.path
        ));
    }
    if !root.fstype.is_empty() && root.fstype != "ext4" {
        errors.push(format!(
            "the selected root partition must be ext4 or blank, found {}",
            root.fstype
        ));
    }
    if root.size_bytes < MIN_ROOT_BYTES {
        errors.push(format!(
            "the selected root partition is too small: {} GiB (minimum {} GiB)",
            root.size_bytes / 1024 / 1024 / 1024,
            MIN_ROOT_BYTES / 1024 / 1024 / 1024
        ));
    }

    if let Some(esp_ref) = plan.esp_partition.as_ref() {
        if let Some(esp) = find_partition(target, esp_ref) {
            if esp.path == root.path {
                errors.push("root and EFI system partitions must differ".into());
            }
            if esp.fstype != "vfat" {
                errors.push(format!(
                    "the EFI system partition must be vfat, found {}",
                    esp.fstype
                ));
            }
            if !esp.mountpoints.is_empty() {
                warnings.push("the existing EFI system partition is mounted; it will be reused, not formatted".into());
            }
        } else {
            errors.push(format!(
                "EFI system partition {} was not found on {}",
                esp_ref.path, target.path
            ));
        }
    }

    errors.extend(preserve_errors(plan, target, Some(&root.path), true));
    if target.children.iter().any(is_apfs) {
        warnings.push("APFS/HFS partitions are preserved and will not be modified".into());
    }
}

fn validate_manual(
    plan: &InstallPlan,
    target: &DiskInfo,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    if target.free_extents.is_empty() {
        errors.push(
            "could not read free space on the target (parted is missing or the table is unreadable)"
                .into(),
        );
    }
    let Some(chosen) = plan.layout.free_extent.as_ref() else {
        errors.push("manual mode requires the chosen free space".into());
        return;
    };
    let Some(extent) = target
        .free_extents
        .iter()
        .find(|extent| extent.start_bytes == chosen.start_bytes)
    else {
        errors.push("the chosen free space is no longer available; refresh the disk list".into());
        return;
    };
    if extent.size_bytes < chosen.size_bytes {
        errors.push("the chosen free space changed; refresh the disk list".into());
    }
    let available = extent.size_bytes.saturating_sub(PLACEMENT_GUARD_BYTES);
    let fixed_bytes: u64 = plan
        .layout
        .partitions
        .iter()
        .filter_map(|partition| partition.size_bytes)
        .sum();
    if fixed_bytes > available {
        errors.push(format!(
            "the planned partitions need {} GiB but the chosen free space holds {} GiB",
            fixed_bytes / 1024 / 1024 / 1024,
            available / 1024 / 1024 / 1024
        ));
    }

    errors.extend(preserve_errors(plan, target, None, true));
    if target.children.iter().any(is_apfs)
        && !target.children.iter().all(|child| {
            plan.layout
                .preserve
                .iter()
                .any(|partition| partition_ref_matches(child, partition))
        })
    {
        errors.push("manual mode must preserve every APFS/HFS partition".into());
    }
    if target.children.iter().any(is_apfs) {
        warnings.push("APFS/HFS partitions are preserved; free-space layout only".into());
    }
    if target.pttype.is_empty() {
        warnings.push("the target has no partition table; the engine will create GPT".into());
    }
}

fn validate_nvidia(
    plan: &InstallPlan,
    hardware: &HardwareInfo,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    if !plan.nvidia.install_driver && !plan.nvidia.install_cuda {
        return;
    }

    let nvidia: Vec<&GpuInfo> = hardware.gpus.iter().filter(|gpu| gpu.is_nvidia).collect();
    if nvidia.is_empty() {
        errors.push("NVIDIA options were selected but no PCI 10de device was detected".into());
    }
    if nvidia.iter().any(|gpu| gpu.is_datacenter) {
        errors.push(
            "this NVIDIA data-center GPU is not supported by the consumer driver path".into(),
        );
    }
    if hardware.secure_boot && !plan.nvidia.secure_boot_mok {
        errors.push("Secure Boot is enabled; acknowledge the MOK enrollment step".into());
    }
    if nvidia.iter().any(|gpu| gpu.is_hybrid) {
        warnings.push(
            "hybrid NVIDIA graphics detected: the integrated GPU stays enabled and PRIME/offload \
             must be used"
                .into(),
        );
    }
    if plan.nvidia.install_driver
        && !plan.licenses.iter().any(|license| {
            license.id == shiny_installer_protocol::NVIDIA_DRIVER_LICENSE_ID
                && license.version == shiny_installer_protocol::NVIDIA_DRIVER_LICENSE_VERSION
        })
    {
        errors.push("the NVIDIA driver license version is missing or not accepted".into());
    }
    if plan.nvidia.install_cuda
        && !plan.licenses.iter().any(|license| {
            license.id == shiny_installer_protocol::CUDA_LICENSE_ID
                && !license.version.trim().is_empty()
        })
    {
        errors.push("the CUDA license version is missing or not accepted".into());
    }
}

fn preserve_errors(
    plan: &InstallPlan,
    target: &DiskInfo,
    modified: Option<&str>,
    require_all: bool,
) -> Vec<String> {
    let mut errors = Vec::new();
    for partition in &plan.layout.preserve {
        if !target
            .children
            .iter()
            .any(|child| partition_ref_matches(child, partition))
        {
            errors.push(format!(
                "preserved partition {} was not found on {}",
                partition.path, target.path
            ));
        }
        if modified == Some(partition.path.as_str()) {
            errors.push("the selected root partition cannot be preserved".into());
        }
    }
    if require_all {
        for child in &target.children {
            if modified == Some(child.path.as_str()) {
                continue;
            }
            if !plan
                .layout
                .preserve
                .iter()
                .any(|partition| partition_ref_matches(child, partition))
            {
                errors.push(format!(
                    "existing partition {} must be listed as preserved",
                    child.path
                ));
            }
        }
    }
    errors
}

fn find_partition<'a>(target: &'a DiskInfo, reference: &PartitionRef) -> Option<&'a DiskInfo> {
    target
        .children
        .iter()
        .find(|child| partition_ref_matches(child, reference))
}

fn partition_ref_matches(partition: &DiskInfo, reference: &PartitionRef) -> bool {
    if !reference.path.is_empty() && partition.path != reference.path {
        return false;
    }
    if !reference.partuuid.is_empty() && partition.partuuid != reference.partuuid {
        return false;
    }
    if !reference.uuid.is_empty() && partition.uuid != reference.uuid {
        return false;
    }
    true
}

fn is_apfs(partition: &DiskInfo) -> bool {
    let fstype = partition.fstype.to_ascii_lowercase();
    let typename = partition.parttypename.to_ascii_lowercase();
    fstype == "apfs"
        || fstype == "hfs"
        || fstype == "hfsplus"
        || typename.contains("apple apfs")
        || typename.contains("apple hfs")
}

fn is_foreign_root(partition: &DiskInfo) -> bool {
    let fstype = partition.fstype.to_ascii_lowercase();
    let typename = partition.parttypename.to_ascii_lowercase();
    !partition.holders.is_empty()
        || matches!(
            fstype.as_str(),
            "vfat"
                | "fat32"
                | "swap"
                | "apfs"
                | "hfs"
                | "hfsplus"
                | "ntfs"
                | "bitlocker"
                | "crypto_luks"
                | "lvm2_member"
                | "linux_raid_member"
        )
        || typename.contains("efi system")
        || typename.contains("linux swap")
        || typename.contains("lvm")
        || typename.contains("raid")
        || typename.contains("bitlocker")
}

fn parse_disk(node: &Value) -> DiskInfo {
    let name = node
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let path = node
        .get("path")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("/dev/{name}"));
    let kind = node
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("disk")
        .to_string();
    let children = node
        .get("children")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(parse_disk).collect())
        .unwrap_or_default();

    DiskInfo {
        name: name.clone(),
        path,
        size_bytes: parse_u64(node.get("size")),
        start_bytes: 0,
        model: string_field(node, "model"),
        serial: string_field(node, "serial"),
        tran: string_field(node, "tran"),
        removable: parse_bool(node.get("rm")),
        read_only: parse_bool(node.get("ro")),
        is_loop: kind == "loop",
        is_rom: kind == "rom",
        mountpoints: parse_mountpoints(node),
        fstype: string_field(node, "fstype"),
        pttype: string_field(node, "pttype"),
        partuuid: string_field(node, "partuuid"),
        uuid: string_field(node, "uuid"),
        label: string_field(node, "label"),
        fsver: string_field(node, "fsver"),
        parttypename: string_field(node, "parttypename"),
        partflags: parse_flags(node.get("partflags")),
        holders: read_holders(&name),
        free_extents: Vec::new(),
        children,
        kind,
    }
}

fn parse_flags(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        Some(Value::String(value)) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn read_holders(name: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(format!("/sys/class/block/{name}/holders")) else {
        return Vec::new();
    };
    let mut holders: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    holders.sort();
    holders
}

#[derive(Default)]
struct DiskLayout {
    free: Vec<FreeExtent>,
    starts: BTreeMap<u32, u64>,
}

/// Read the partition starts and the free extents with a single read-only
/// `parted` call. Failures yield an empty layout; callers keep working with
/// the `lsblk` data and the UI marks the positions as approximate.
fn read_disk_layout(path: &str) -> DiskLayout {
    let Ok(output) = Command::new("parted")
        .args(["-m", "-s", path, "unit", "B", "print", "free"])
        .output()
    else {
        return DiskLayout::default();
    };
    if !output.status.success() {
        return DiskLayout::default();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_parted_layout(&text)
}

fn parse_parted_layout(text: &str) -> DiskLayout {
    let mut layout = DiskLayout::default();
    for line in text.lines() {
        if line.starts_with("BYT") || line.starts_with("WARNING") || line.starts_with("Error") {
            continue;
        }
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() < 5 {
            continue;
        }
        let Ok(number) = fields[0].trim().parse::<u32>() else {
            continue;
        };
        let start = parse_bytes(fields[1]);
        let end = parse_bytes(fields[2]);
        let size = parse_bytes(fields[3]).max(end.saturating_sub(start));
        if fields[4].trim_end_matches(';') == "free" {
            if size > 0 {
                layout.free.push(FreeExtent {
                    start_bytes: start,
                    size_bytes: size,
                });
            }
        } else {
            layout.starts.insert(number, start);
        }
    }
    layout
}

/// Partition number from an `lsblk` name, using the whole-disk name:
/// `sda2 -> 2`, `nvme0n1p2 -> 2`, `mmcblk0p1 -> 1`.
fn partition_number(name: &str, disk: &str) -> Option<u32> {
    let rest = name.strip_prefix(disk)?;
    let digits = rest.strip_prefix('p').unwrap_or(rest);
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

fn parse_bytes(value: &str) -> u64 {
    value
        .trim()
        .trim_end_matches(';')
        .trim_end_matches('B')
        .parse()
        .unwrap_or(0)
}

fn secure_boot_enabled() -> bool {
    for line in command_lines("mokutil", &["--sb-state"]) {
        let line = line.to_ascii_lowercase();
        if line.contains("secureboot enabled") {
            return true;
        }
        if line.contains("secureboot disabled") {
            return false;
        }
    }
    let Ok(entries) = fs::read_dir("/sys/firmware/efi/efivars") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("SecureBoot-") {
            continue;
        }
        if let Ok(data) = fs::read(entry.path()) {
            if data.last().copied() == Some(1) {
                return true;
            }
        }
    }
    false
}

fn detect_gpus() -> Vec<GpuInfo> {
    let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") else {
        return Vec::new();
    };
    let mut gpus = Vec::new();
    for entry in entries.flatten() {
        let base = entry.path();
        let vendor = read_path_trim(&base.join("vendor"));
        let device = read_path_trim(&base.join("device"));
        let class = read_path_trim(&base.join("class"));
        if !class.starts_with("0x03") && vendor != "0x10de" {
            continue;
        }
        let slot = entry.file_name().to_string_lossy().to_string();
        let model =
            pci_model(&slot).unwrap_or_else(|| format!("{} {}", vendor_name(&vendor), device));
        let is_nvidia = vendor == "0x10de";
        let is_datacenter = is_nvidia && datacenter_model(&model);
        gpus.push(GpuInfo {
            vendor: vendor_name(&vendor).into(),
            model,
            pci_id: slot,
            is_nvidia,
            is_hybrid: false,
            is_datacenter,
        });
    }
    let has_nvidia = gpus.iter().any(|gpu| gpu.is_nvidia);
    let has_other_display = gpus
        .iter()
        .any(|gpu| !gpu.is_nvidia && gpu.vendor != "Apple");
    if has_nvidia && has_other_display {
        for gpu in &mut gpus {
            if gpu.is_nvidia {
                gpu.is_hybrid = true;
            }
        }
    }
    gpus.sort_by(|a, b| a.pci_id.cmp(&b.pci_id));
    gpus
}

fn read_path_trim(path: &Path) -> String {
    fs::read_to_string(path)
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

fn vendor_name(vendor: &str) -> &'static str {
    match vendor {
        "0x10de" => "NVIDIA",
        "0x8086" => "Intel",
        "0x1002" => "AMD",
        "0x106b" => "Apple",
        _ => "Unknown",
    }
}

fn pci_model(slot: &str) -> Option<String> {
    let output = Command::new("lspci")
        .args(["-Dnn", "-s", slot])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&output.stdout);
    let line = line.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    Some(
        line.split_once("]: ")
            .map(|(_, model)| model.to_string())
            .unwrap_or_else(|| line.to_string()),
    )
}

fn datacenter_model(model: &str) -> bool {
    let model = model.to_ascii_uppercase();
    [
        "TESLA",
        "DATA CENTER",
        "A100",
        "H100",
        "H200",
        "L40",
        "L4",
        "A10",
        "A16",
        "A30",
        "A40",
        "V100",
        "P100",
        "T4",
    ]
    .iter()
    .any(|token| model.contains(token))
}

fn string_field(node: &Value, key: &str) -> String {
    node.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn parse_u64(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0),
        Some(Value::String(text)) => text.parse().unwrap_or(0),
        _ => 0,
    }
}

fn parse_bool(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0) != 0,
        Some(Value::String(text)) => matches!(text.as_str(), "1" | "true" | "yes"),
        _ => false,
    }
}

fn parse_mountpoints(node: &Value) -> Vec<String> {
    if let Some(points) = node.get("mountpoints").and_then(Value::as_array) {
        return points
            .iter()
            .filter_map(Value::as_str)
            .filter(|point| !point.is_empty())
            .map(str::to_string)
            .collect();
    }
    node.get("mountpoint")
        .and_then(Value::as_str)
        .filter(|point| !point.is_empty())
        .map(|point| vec![point.to_string()])
        .unwrap_or_default()
}

pub fn system_options() -> SystemOptions {
    let default_locale = locale_default();
    let default_keyboard = keyboard_default();
    let default_timezone = timezone_default();

    let mut locales = BTreeSet::new();
    for line in command_lines("localectl", &["list-locales"]) {
        if let Some(locale) = canonical_locale(&line) {
            locales.insert(locale);
        }
    }
    if let Ok(text) = fs::read_to_string("/usr/share/i18n/SUPPORTED") {
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            let Some(locale) = fields.next() else {
                continue;
            };
            if fields.next() != Some("UTF-8") {
                continue;
            }
            if let Some(locale) = canonical_locale(locale) {
                locales.insert(locale);
            }
        }
    }
    for line in command_lines("locale", &["-a"]) {
        if let Some(locale) = canonical_locale(&line) {
            locales.insert(locale);
        }
    }
    if let Some(locale) = canonical_locale(&default_locale) {
        locales.insert(locale);
    }
    let mut locales: Vec<String> = locales.into_iter().collect();
    locales.sort();
    if locales.is_empty() {
        locales.push(default_locale.clone());
    }

    let mut keyboards = BTreeSet::new();
    for line in command_lines("localectl", &["list-keymaps"]) {
        let name = line.trim();
        if !name.is_empty() {
            keyboards.insert(name.to_string());
        }
    }
    for path in [
        "/usr/share/X11/xkb/rules/evdev.lst",
        "/usr/share/X11/xkb/rules/base.lst",
    ] {
        if let Ok(text) = fs::read_to_string(path) {
            parse_xkb_layouts(&text, &mut keyboards);
        }
    }
    if !default_keyboard.is_empty() {
        keyboards.insert(default_keyboard.clone());
    }
    let mut keyboards: Vec<String> = keyboards.into_iter().collect();
    keyboards.sort();
    if keyboards.is_empty() {
        keyboards = vec!["us".into(), "it".into(), "gb".into()];
    }

    let mut timezones = BTreeSet::new();
    for line in command_lines("timedatectl", &["list-timezones"]) {
        let zone = line.trim();
        if !zone.is_empty() {
            timezones.insert(zone.to_string());
        }
    }
    if let Ok(text) = fs::read_to_string("/usr/share/zoneinfo/zone.tab") {
        for line in text.lines() {
            if line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            if let Some(zone) = fields.get(2) {
                timezones.insert((*zone).to_string());
            }
        }
    }
    if !default_timezone.is_empty() {
        timezones.insert(default_timezone.clone());
    }
    let mut timezones: Vec<String> = timezones.into_iter().collect();
    timezones.sort();
    if timezones.is_empty() {
        timezones.push("UTC".into());
    }

    SystemOptions {
        locales,
        keyboards,
        timezones,
        default_locale: canonical_locale(&default_locale).unwrap_or(default_locale),
        default_keyboard,
        default_timezone,
    }
}

fn command_lines(command: &str, args: &[&str]) -> Vec<String> {
    let Ok(output) = Command::new(command).args(args).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn canonical_locale(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || matches!(value, "C" | "POSIX") {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    if lower.ends_with(".utf8") || lower.ends_with(".utf-8") {
        let base = value.split('.').next().unwrap_or(value);
        return Some(format!("{base}.UTF-8"));
    }
    if value.contains('.') {
        return None;
    }
    Some(format!("{value}.UTF-8"))
}

fn locale_default() -> String {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return canonical_locale(value).unwrap_or_else(|| value.to_string());
            }
        }
    }
    "en_US.UTF-8".into()
}

fn keyboard_default() -> String {
    if let Ok(value) = std::env::var("XKB_DEFAULT_LAYOUT") {
        let value = value.trim();
        if !value.is_empty() {
            return value.split(',').next().unwrap_or(value).to_string();
        }
    }
    if let Ok(text) = fs::read_to_string("/etc/default/keyboard") {
        for line in text.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("XKBLAYOUT=") {
                let value = value.trim_matches('"').trim_matches('\'');
                if !value.is_empty() {
                    return value.split(',').next().unwrap_or(value).to_string();
                }
            }
        }
    }
    for line in command_lines("localectl", &["status"]) {
        if let Some(value) = line.strip_prefix("X11 Layout:") {
            let value = value.trim();
            if !value.is_empty() {
                return value.split(',').next().unwrap_or(value).to_string();
            }
        }
    }
    "us".into()
}

fn timezone_default() -> String {
    for line in command_lines("timedatectl", &["show", "-p", "Timezone", "--value"]) {
        if !line.is_empty() {
            return line;
        }
    }
    if let Ok(value) = fs::read_to_string("/etc/timezone") {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    "UTC".into()
}

fn parse_xkb_layouts(text: &str, layouts: &mut BTreeSet<String>) {
    let mut in_layout = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('!') {
            in_layout = line == "! layout";
            continue;
        }
        if !in_layout || line.is_empty() {
            continue;
        }
        if let Some(name) = line.split_whitespace().next() {
            if !name.starts_with('#') {
                layouts.insert(name.to_string());
            }
        }
    }
}

fn read_trim(path: &str) -> String {
    fs::read_to_string(path)
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

fn detects_t2() -> bool {
    let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") else {
        return false;
    };
    for entry in entries.flatten() {
        let base = entry.path();
        let vendor = fs::read_to_string(base.join("vendor")).unwrap_or_default();
        let device = fs::read_to_string(base.join("device")).unwrap_or_default();
        let vendor = vendor.trim();
        let device = device.trim();
        if vendor == "0x106b" && (device == "0x1801" || device == "0x1802") {
            return true;
        }
    }
    false
}

fn live_medium_device() -> Option<String> {
    let mountinfo = fs::read_to_string("/proc/self/mountinfo").ok()?;
    for line in mountinfo.lines() {
        let Some((before, after)) = line.split_once(" - ") else {
            continue;
        };
        let fields: Vec<&str> = before.split_whitespace().collect();
        if fields.get(4).copied() != Some("/run/live/medium") {
            continue;
        }
        let source = after.split_whitespace().next()?;
        if source.starts_with("/dev/") {
            return Some(canonical_or_self(source));
        }
    }
    None
}

fn canonical_or_self(path: &str) -> String {
    fs::canonicalize(Path::new(path))
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parted_layout_reads_partition_starts_and_free_extents() {
        let sample = "BYT;\n\
            /dev/nvme0n1:2000B:scsi:512:4096:gpt:ATA:;\n\
            1:1048576B:537919487B:536870912B:fat32:EFI system partition:boot, esp;\n\
            2:537919488B:1073741823B:535822336B:ext4::;\n\
            1:1073741824B:1999999999B:926257176B:free;\n";
        let layout = parse_parted_layout(sample);
        assert_eq!(layout.starts.get(&1), Some(&1_048_576));
        assert_eq!(layout.starts.get(&2), Some(&537_919_488));
        assert_eq!(layout.free.len(), 1);
        assert_eq!(layout.free[0].start_bytes, 1_073_741_824);
        assert_eq!(layout.free[0].size_bytes, 926_258_175);
    }

    #[test]
    fn partition_numbers_follow_the_whole_disk_name() {
        assert_eq!(partition_number("sda2", "sda"), Some(2));
        assert_eq!(partition_number("nvme0n1p12", "nvme0n1"), Some(12));
        assert_eq!(partition_number("mmcblk0p1", "mmcblk0"), Some(1));
        assert_eq!(partition_number("vdb3", "vdb"), Some(3));
        assert_eq!(partition_number("sda", "sda"), None);
    }

    #[test]
    fn locale_normalization_handles_utf8_and_c() {
        assert_eq!(canonical_locale("en_US.utf8"), Some("en_US.UTF-8".into()));
        assert_eq!(canonical_locale("de_DE.UTF-8"), Some("de_DE.UTF-8".into()));
        assert_eq!(canonical_locale("C"), None);
        assert_eq!(canonical_locale("POSIX"), None);
    }

    #[test]
    fn datacenter_gpu_heuristic_is_conservative() {
        assert!(datacenter_model("NVIDIA A100-SXM4-80GB"));
        assert!(datacenter_model("Tesla T4"));
        assert!(!datacenter_model("NVIDIA GeForce RTX 3060"));
    }

    #[test]
    fn xkb_layout_section_is_parsed() {
        let sample =
            "! layout\n  us  English (US)\n  it  Italian\n! variant\n  intl us: International\n";
        let mut layouts = BTreeSet::new();
        parse_xkb_layouts(sample, &mut layouts);
        assert!(layouts.contains("us"));
        assert!(layouts.contains("it"));
        assert!(!layouts.contains("intl"));
    }
}
