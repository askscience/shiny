//! Destructive install engine, separate from the read-only probe/validation.
//!
//! Every command is invoked with an argument vector (never a shell). The
//! helper validates the typed plan and target identity before this module is
//! called. The live source is the read-only squashfs on the live medium.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use shiny_installer_protocol::{
    InstallPlan, PartitionSpec, AI_TIER_LOCAL, AI_TIER_NONE, AI_TIER_OFF, AI_TIER_VOICE,
    MODE_ERASE, MODE_EXISTING, MODE_MANUAL,
};

const TARGET_MOUNT: &str = "/run/shiny-installer/target";
const SOURCE_MOUNT: &str = "/run/shiny-installer/source";
const MIB: u64 = 1024 * 1024;

pub fn apply(
    plan: &InstallPlan,
    allow_loop: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&str, u8),
) -> Result<(), String> {
    if plan.nvidia.install_driver || plan.nvidia.install_cuda {
        return Err("NVIDIA target installation is not implemented in this engine".into());
    }
    if !allow_loop && plan.target.path.starts_with("/dev/loop") {
        return Err("loop devices are refused by the production installer".into());
    }

    let source_squash = std::env::var("LITEAURA_ENGINE_SQUASHFS")
        .unwrap_or_else(|_| "/run/live/medium/live/filesystem.squashfs".into());
    let source_override = std::env::var("LITEAURA_ENGINE_SOURCE").ok();
    let medium = std::env::var("LITEAURA_ENGINE_MEDIUM")
        .unwrap_or_else(|_| "/run/live/medium/live".into());

    let mut cleanup = Cleanup::default();
    let source = if let Some(path) = source_override {
        PathBuf::from(path)
    } else {
        let path = PathBuf::from(SOURCE_MOUNT);
        fs::create_dir_all(&path).map_err(|e| format!("source mount dir: {e}"))?;
        run("mount", &["-o", "loop,ro", &source_squash, SOURCE_MOUNT])?;
        cleanup.mounts.push(path.clone());
        path
    };
    if !source.is_dir() {
        return Err(format!("source rootfs is not a directory: {}", source.display()));
    }

    let target_mount = PathBuf::from(TARGET_MOUNT);
    fs::create_dir_all(&target_mount).map_err(|e| format!("target mount dir: {e}"))?;

    let disks = partitions(plan)?;
    progress("prepare", 3);
    check_cancel(cancel)?;

    progress("partition", 12);
    check_cancel(cancel)?;

    if plan.mode == MODE_ERASE {
        run("wipefs", &["-a", &plan.target.path])?;
        run("parted", &["-s", &plan.target.path, "mklabel", "gpt"])?;
    }

    for part in &disks {
        if part.create {
            create_partition(plan, part)?;
        }
    }
    run("partprobe", &[&plan.target.path])?;
    let _ = run("udevadm", &["settle"]);
    progress("format", 28);
    check_cancel(cancel)?;

    for part in &disks {
        format_partition(plan, part)?;
    }

    let root = disks.iter().find(|p| p.role == "root").ok_or("no root partition planned")?;
    let esp = disks.iter().find(|p| p.role == "esp").ok_or("no ESP planned")?;
    let home = disks.iter().find(|p| p.role == "home");
    let swap = disks.iter().find(|p| p.role == "swap");

    fs::create_dir_all(&target_mount).map_err(|e| format!("target mount dir: {e}"))?;
    mount(&target_mount, &root.path, "ext4")?;
    cleanup.mounts.push(target_mount.clone());
    let esp_mount = target_mount.join("boot/efi");
    fs::create_dir_all(&esp_mount).map_err(|e| format!("ESP mount dir: {e}"))?;
    mount(&esp_mount, &esp.path, "vfat")?;
    cleanup.mounts.push(esp_mount.clone());

    if let Some(home) = home {
        let hm = target_mount.join("home");
        fs::create_dir_all(&hm).map_err(|e| format!("home mount dir: {e}"))?;
        mount(&hm, &home.path, "ext4")?;
        cleanup.mounts.push(hm);
    }
    if let Some(swap) = swap {
        run("swapon", &[&swap.path])?;
        cleanup.swaps.push(swap.path.clone());
    }

    progress("unpack", 42);
    check_cancel(cancel)?;
    run(
        "rsync",
        &[
            "-aHAX",
            "--numeric-ids",
            "--one-file-system",
            "--exclude=/proc",
            "--exclude=/sys",
            "--exclude=/dev",
            "--exclude=/run",
            "--exclude=/tmp",
            "--exclude=/var/tmp",
            "--exclude=/mnt",
            "--exclude=/media",
            &format!("{}/", source.display()),
            &format!("{}/", target_mount.display()),
        ],
    )?;

    progress("configure", 68);
    check_cancel(cancel)?;
    configure_target(plan, &target_mount, &root.path, &esp.path, home, swap, &medium)?;

    progress("bootloader", 88);
    check_cancel(cancel)?;
    install_bootloader(plan, &target_mount, &root.path, &esp_mount, &medium)?;
    unmount_chroot(&target_mount);

    progress("finish", 96);
    run("sync", &[])?;
    cleanup.run();
    progress("done", 100);
    Ok(())
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err("cancelled".into())
    } else {
        Ok(())
    }
}

#[derive(Default)]
struct Cleanup {
    mounts: Vec<PathBuf>,
    swaps: Vec<String>,
}

impl Cleanup {
    fn run(&mut self) {
        for swap in self.swaps.drain(..).rev() {
            let _ = Command::new("swapoff").arg(swap).output();
        }
        for mount in self.mounts.drain(..).rev() {
            let _ = Command::new("umount").arg(&mount).output();
            if mount.exists() {
                let _ = Command::new("umount").args(["-l"]).arg(&mount).output();
            }
        }
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        self.run();
    }
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "{} {:?} failed ({}): {}",
            program,
            args,
            output.status,
            if stderr.is_empty() { stdout } else { stderr }
        ));
    }
    Ok(stdout)
}

fn mount(target: &Path, source: &str, fstype: &str) -> Result<(), String> {
    run(
        "mount",
        &[
            "-t",
            fstype,
            source,
            target.to_str().ok_or("mount path is not UTF-8")?,
        ],
    )?;
    Ok(())
}

struct PlannedPart {
    role: String,
    spec: PartitionSpec,
    path: String,
    number: u32,
    start_mib: u64,
    end_mib: Option<u64>,
    create: bool,
}

fn partitions(plan: &InstallPlan) -> Result<Vec<PlannedPart>, String> {
    match plan.mode.as_str() {
        MODE_EXISTING => {
            let root = plan.target_partition.as_ref().ok_or("root partition missing")?;
            let esp = plan.esp_partition.as_ref().ok_or("ESP missing")?;
            Ok(vec![
                PlannedPart {
                    role: "root".into(),
                    spec: PartitionSpec {
                        role: "root".into(),
                        fs: root.fstype.clone(),
                        ..PartitionSpec::default()
                    },
                    path: root.path.clone(),
                    number: 0,
                    start_mib: 0,
                    end_mib: None,
                    create: false,
                },
                PlannedPart {
                    role: "esp".into(),
                    spec: PartitionSpec {
                        role: "esp".into(),
                        fs: esp.fstype.clone(),
                        ..PartitionSpec::default()
                    },
                    path: esp.path.clone(),
                    number: 0,
                    start_mib: 0,
                    end_mib: None,
                    create: false,
                },
            ])
        }
        MODE_ERASE | MODE_MANUAL => {
            let mut out = Vec::new();
            let mut next = if plan.mode == MODE_MANUAL {
                max_partition_number(&plan.target.path)? + 1
            } else {
                1
            };
            let extent_end = if plan.mode == MODE_MANUAL {
                Some(free_extent_end_mib(plan)?)
            } else {
                None
            };
            let mut cursor = if plan.mode == MODE_MANUAL {
                free_space_start(plan)?
            } else {
                1
            };
            for spec in &plan.layout.partitions {
                let size_mib = spec.size_bytes.map(|b| b.div_ceil(MIB)).unwrap_or(0);
                let number = next;
                let start = cursor;
                let end = if spec.role == "root" && plan.mode == MODE_ERASE {
                    None
                } else {
                    if size_mib == 0 {
                        return Err(format!("{} partition has no size", spec.role));
                    }
                    cursor += size_mib;
                    Some(cursor)
                };
                out.push(PlannedPart {
                    role: spec.role.clone(),
                    path: part_path(&plan.target.path, number),
                    number,
                    start_mib: start,
                    end_mib: end,
                    spec: spec.clone(),
                    create: true,
                });
                next += 1;
            }
            if let Some(end) = extent_end {
                if cursor > end {
                    return Err(format!(
                        "the planned partitions end at {cursor} MiB but the chosen free space ends at {end} MiB"
                    ));
                }
            }
            Ok(out)
        }
        other => Err(format!("unsupported install mode: {other}")),
    }
}

fn create_partition(plan: &InstallPlan, part: &PlannedPart) -> Result<(), String> {
    let start = part.start_mib.to_string();
    if let Some(end) = part.end_mib {
        let end = end.to_string();
        run(
            "parted",
            &["-s", &plan.target.path, "unit", "MiB", "mkpart", &part.role, parted_fs(&part.spec.fs), &start, &end],
        )?;
    } else {
        run(
            "parted",
            &["-s", &plan.target.path, "unit", "MiB", "mkpart", &part.role, parted_fs(&part.spec.fs), &start, "100%"],
        )?;
    }
    if part.role == "esp" {
        run(
            "parted",
            &["-s", &plan.target.path, "set", &part.number.to_string(), "esp", "on"],
        )?;
    }
    Ok(())
}

fn format_partition(plan: &InstallPlan, part: &PlannedPart) -> Result<(), String> {
    match part.role.as_str() {
        "esp" => {
            if plan.mode != MODE_EXISTING {
                run("mkfs.vfat", &["-F32", "-n", "PEAKESP", &part.path])?;
            }
        }
        "swap" => {
            run("mkswap", &["-L", "PEAKSWAP", &part.path])?;
        }
        "root" | "home" => {
            let label = if part.role == "root" { "PEAKROOT" } else { "PEAKHOME" };
            run("mkfs.ext4", &["-F", "-L", label, &part.path])?;
        }
        _ => {}
    }
    Ok(())
}

fn max_partition_number(disk: &str) -> Result<u32, String> {
    let output = run("parted", &["-m", "-s", disk, "unit", "MiB", "print"])?;
    let mut max = 0;
    for line in output.lines() {
        let Some(first) = line.split(':').next() else { continue };
        if let Ok(number) = first.trim().parse::<u32>() {
            max = max.max(number);
        }
    }
    Ok(max)
}

fn part_path(disk: &str, number: u32) -> String {
    if disk.starts_with("/dev/nvme") || disk.starts_with("/dev/mmcblk") || disk.starts_with("/dev/loop") {
        format!("{disk}p{number}")
    } else {
        format!("{disk}{number}")
    }
}

fn parted_fs(fs: &str) -> &str {
    match fs {
        "vfat" => "fat32",
        "swap" => "linux-swap",
        other => other,
    }
}

/// First MiB-aligned sector inside the free extent chosen by the user.
fn free_space_start(plan: &InstallPlan) -> Result<u64, String> {
    let extent = plan
        .layout
        .free_extent
        .as_ref()
        .ok_or("manual mode requires the chosen free space")?;
    Ok(extent.start_bytes.div_ceil(MIB).max(1))
}

/// Last MiB of the chosen free extent, used to check the planned end.
fn free_extent_end_mib(plan: &InstallPlan) -> Result<u64, String> {
    let extent = plan
        .layout
        .free_extent
        .as_ref()
        .ok_or("manual mode requires the chosen free space")?;
    Ok(extent.start_bytes.saturating_add(extent.size_bytes) / MIB)
}

fn configure_target(
    plan: &InstallPlan,
    target: &Path,
    root: &str,
    esp: &str,
    home: Option<&PlannedPart>,
    swap: Option<&PlannedPart>,
    medium: &str,
) -> Result<(), String> {
    let root_uuid = uuid_of(root)?;
    let esp_uuid = uuid_of(esp)?;
    let mut fstab = format!(
        "UUID={root_uuid} / ext4 defaults 0 1\nUUID={esp_uuid} /boot/efi vfat umask=0077 0 1\n"
    );
    if let Some(home) = home {
        fstab.push_str(&format!("UUID={} /home ext4 defaults 0 2\n", uuid_of(&home.path)?));
    }
    if let Some(swap) = swap {
        fstab.push_str(&format!("UUID={} none swap sw 0 0\n", uuid_of(&swap.path)?));
    }
    write(target, "etc/fstab", &fstab)?;
    write(target, "etc/hostname", &format!("{}\n", plan.hostname))?;
    write(
        target,
        "etc/hosts",
        &format!("127.0.0.1 localhost\n127.0.1.1 {}\n", plan.hostname),
    )?;

    for (dir, mode) in [
        ("proc", 0o555),
        ("sys", 0o555),
        ("dev", 0o755),
        ("run", 0o755),
        ("tmp", 0o1777),
        ("var/tmp", 0o1777),
        ("var/log", 0o755),
        ("var/lib/initramfs-tools", 0o755),
        ("home", 0o755),
    ] {
        let path = target.join(dir);
        fs::create_dir_all(&path).map_err(|e| format!("create {}: {e}", path.display()))?;
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(mode));
    }

    mount_chroot(target)?;
    let useradd = run(
        "chroot",
        &[
            target.to_str().ok_or("target path")?,
            "/usr/sbin/useradd",
            "-m",
            "-s",
            "/bin/bash",
            "-c",
            &plan.full_name,
            &plan.username,
        ],
    );
    if let Err(error) = useradd {
        if !error.contains("already exists") {
            return Err(error);
        }
    }
    let _ = run(
        "chroot",
        &[
            target.to_str().ok_or("target path")?,
            "/usr/sbin/usermod",
            "-aG",
            "sudo,video,audio,bluetooth,shiny",
            &plan.username,
        ],
    );
    let mut child = Command::new("chroot")
        .args([target.to_str().ok_or("target path")?, "/usr/sbin/chpasswd"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("chpasswd: {e}"))?;
    child
        .stdin
        .as_mut()
        .ok_or("chpasswd stdin")?
        .write_all(format!("{}:{}\n", plan.username, plan.password).as_bytes())
        .map_err(|e| format!("write password: {e}"))?;
    let status = child.wait().map_err(|e| format!("chpasswd wait: {e}"))?;
    if !status.success() {
        return Err("chpasswd failed".into());
    }

    write(target, "etc/default/locale", &format!("LANG={}\n", plan.locale))?;
    let locale_gen = target.join("etc/locale.gen");
    let mut locale_text = fs::read_to_string(&locale_gen).unwrap_or_default();
    if !locale_text.contains(&plan.locale) {
        locale_text.push_str(&format!("{} UTF-8\n", plan.locale));
        fs::write(&locale_gen, locale_text).map_err(|e| format!("locale.gen: {e}"))?;
    }
    let _ = run(
        "chroot",
        &[target.to_str().ok_or("target path")?, "/usr/sbin/locale-gen"],
    );
    write(
        target,
        "etc/default/keyboard",
        &format!("XKBMODEL=\"pc105\"\nXKBLAYOUT=\"{}\"\nXKBVARIANT=\"\"\nXKBOPTIONS=\"\"\n", plan.keyboard),
    )?;
    write(target, "etc/timezone", &format!("{}\n", plan.timezone))?;
    let zone = target.join("usr/share/zoneinfo").join(&plan.timezone);
    if zone.exists() {
        let _ = fs::remove_file(target.join("etc/localtime"));
        std::os::unix::fs::symlink(zone, target.join("etc/localtime"))
            .map_err(|e| format!("localtime: {e}"))?;
    }

    // The installed system signs in on LiteAura's own login screen (the
    // packaged seat, `liteaura-seat`), not through the live image's LightDM
    // autologin: the live session, its xsession entry and the autologin
    // drop-in go. The AI tier the user chose is the machine's default, read
    // by every account's session and server.
    write(target, "etc/default/liteaura", &machine_defaults(plan))?;
    for path in [
        "usr/local/bin/liteaura-live-session",
        "usr/local/bin/liteaura-session",
        "usr/share/xsessions/liteaura.desktop",
        "etc/lightdm/lightdm.conf.d/99-liteaura.conf",
        "etc/xdg/autostart/liteaura-live.desktop",
    ] {
        let _ = fs::remove_file(target.join(path));
    }
    remove_ai_payload(plan, target);

    for path in [
        "usr/share/shiny/live/seed.db",
        "usr/libexec/shiny/shiny-installer-helper",
        "usr/lib/shiny/plugins/installer",
        "usr/local/lib/shiny/seed-live-account.sh",
        "etc/systemd/system/shiny-live-seed.service",
        "etc/systemd/system/shiny-installer-helper.service",
        "etc/systemd/system/multi-user.target.wants/shiny-live-seed.service",
        "etc/systemd/system/multi-user.target.wants/shiny-installer-helper.service",
        "etc/systemd/system/multi-user.target.wants/live-config.service",
        "etc/systemd/system/live-config.service",
        "lib/systemd/system/live-config.service",
        "usr/lib/systemd/system/live-config.service",
        "etc/live",
        "lib/live",
        "usr/lib/live",
        "usr/share/live",
        "var/lib/live",
        "var/lib/dbus/machine-id",
    ] {
        let path = target.join(path);
        if path.is_dir() {
            let _ = fs::remove_dir_all(path);
        } else {
            let _ = fs::remove_file(path);
        }
    }
    if let Ok(entries) = fs::read_dir(target.join("usr/lib/systemd/system-generators")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("live-") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    write(target, "etc/machine-id", "\n")?;
    enable_seat(target)?;
    if let Ok(entries) = fs::read_dir(target.join("etc/ssh")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("ssh_host_") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    write(
        target,
        "etc/default/grub",
        "GRUB_DEFAULT=0\nGRUB_TIMEOUT=5\nGRUB_DISTRIBUTOR=\"LiteAura\"\nGRUB_CMDLINE_LINUX=\"quiet rw console=tty0 console=ttyS0,115200\"\nGRUB_CMDLINE_LINUX_DEFAULT=\"quiet rw\"\n",
    )?;

    let version = kernel_version(target, plan.is_t2())?;
    // The T2 kernel arrives in the live rootfs as a package (/boot is
    // populated); the base Debian kernel may only exist on the medium.
    let kernel_dest = target.join(format!("boot/vmlinuz-{version}"));
    if !kernel_dest.exists() {
        let kernel_source = Path::new(medium).join(if plan.is_t2() { "vmlinuz-t2" } else { "vmlinuz" });
        if !kernel_source.exists() {
            return Err(format!("kernel image missing: {}", kernel_source.display()));
        }
        install_file(&kernel_source, &kernel_dest)?;
    }

    // The live initramfs hook would search for a squashfs on the target.
    for path in [
        "usr/share/initramfs-tools/hooks/live",
        "etc/initramfs-tools/conf.d/live",
    ] {
        let _ = fs::remove_dir_all(target.join(path));
        let _ = fs::remove_file(target.join(path));
    }
    if let Ok(entries) = fs::read_dir(target.join("usr/share/initramfs-tools/scripts")) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("live") {
                let _ = fs::remove_dir_all(entry.path());
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    run(
        "chroot",
        &[
            target.to_str().ok_or("target path")?,
            "/usr/sbin/mkinitramfs",
            "-o",
            &format!("/boot/initrd.img-{version}"),
            &version,
        ],
    )?;

    mount_chroot(target)?;
    if plan.is_t2() {
        copy_wireless_firmware(target);
    } else {
        drop_t2_kernel(target);
    }

    // The boot menu follows kernel updates: /etc/liteaura/boot.conf says what
    // this machine boots, and liteaura-update-boot (run here and by the
    // kernel hooks after every apt kernel install/removal) writes
    // /boot/grub/grub.cfg from it. The EFI loader only holds a stub that
    // reads that file (install_bootloader).
    write(target, "etc/liteaura/boot.conf", &boot_conf(&root_uuid, plan.is_t2()))?;
    let updated = run(
        "chroot",
        &[target.to_str().ok_or("target path")?, "/usr/sbin/liteaura-update-boot"],
    );
    if updated.is_err() || !target.join("boot/grub/grub.cfg").exists() {
        // An older package without the updater: one fixed entry.
        write(target, "boot/grub/grub.cfg", &static_grub_cfg(&root_uuid, &version))?;
    }
    write(target, "boot/grub/liteaura-efi.cfg", &efi_stub_cfg(&root_uuid))?;
    Ok(())
}

/// Kernel arguments after `root=`, the same on every installed machine.
const KERNEL_CMDLINE: &str = "rw quiet console=tty0 console=ttyS0,115200";

fn boot_conf(root_uuid: &str, t2: bool) -> String {
    format!(
        "# Written by the LiteAura installer; read by liteaura-update-boot.\n\
         ROOT_UUID={root_uuid}\n\
         FLAVOR={}\n\
         CMDLINE=\"{KERNEL_CMDLINE}\"\n",
        if t2 { "t2" } else { "generic" }
    )
}

fn static_grub_cfg(root_uuid: &str, version: &str) -> String {
    format!(
        "set timeout=5\nset default=0\ninsmod part_gpt\ninsmod ext2\ninsmod search_fs_uuid\n\
         search --no-floppy --fs-uuid --set=root {root_uuid}\n\
         menuentry \"LiteAura\" {{\n linux /boot/vmlinuz-{version} root=UUID={root_uuid} {KERNEL_CMDLINE}\n initrd /boot/initrd.img-{version}\n}}\n"
    )
}

/// The configuration built into the EFI loader: find the root filesystem and
/// read the menu from it, so a kernel update never needs a new loader.
fn efi_stub_cfg(root_uuid: &str) -> String {
    format!(
        "insmod part_gpt\ninsmod ext2\ninsmod search_fs_uuid\ninsmod configfile\n\
         search --no-floppy --fs-uuid --set=root {root_uuid}\n\
         set prefix=($root)/boot/grub\n\
         configfile ($root)/boot/grub/grub.cfg\n"
    )
}

/// T2 Macs: the Wi-Fi/Bluetooth firmware this live session took from the
/// Mac's own macOS (t2-firmware.sh) lives in the RAM overlay, not in the
/// squashfs that was copied. Carry it to the installed system, so erasing
/// macOS does not take Wi-Fi with it. Nothing to copy when it was not found.
const LIVE_FIRMWARE: &str = "/lib/firmware/brcm";

fn copy_wireless_firmware(target: &Path) {
    let dest = target.join("lib/firmware/brcm");
    let Ok(entries) = fs::read_dir(LIVE_FIRMWARE) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_apple_wireless_firmware(&name) {
            continue;
        }
        let to = dest.join(&name);
        if !to.exists() {
            let _ = install_file(&entry.path(), &to);
        }
    }
}

/// The renamed Apple Broadcom files (`brcmfmac4364b3-pcie.apple,borneo.bin`,
/// `brcmbt4377b3-apple,formosa.bin`, …) plus the two board files the renamer
/// keeps under their plain names.
fn is_apple_wireless_firmware(name: &str) -> bool {
    (name.starts_with("brcm") && name.contains("apple"))
        || name == "brcmfmac4364b2-pcie.txt"
        || name == "brcmfmac4364b2-pcie.txcap_blob"
}

/// A generic PC needs neither the T2 kernel nor its repository: purge the
/// kernel packages (their hooks run in the mounted chroot) and drop the
/// source, so apt does not keep updating a kernel this machine never boots.
fn drop_t2_kernel(target: &Path) {
    let Some(root) = target.to_str() else { return };
    let listed = Command::new("chroot")
        .args([root, "dpkg-query", "-W", "-f", "${Package}\n"])
        .output();
    let Ok(listed) = listed else { return };
    let packages: Vec<String> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter(|p| *p == "linux-t2" || ((p.starts_with("linux-image-") || p.starts_with("linux-headers-")) && p.contains("-t2-")))
        .map(str::to_string)
        .collect();
    if !packages.is_empty() {
        let mut args = vec![root, "dpkg", "--purge"];
        args.extend(packages.iter().map(String::as_str));
        let _ = Command::new("chroot").args(&args).output();
    }
    let _ = fs::remove_file(target.join("etc/apt/sources.list.d/t2.list"));
    let _ = fs::remove_file(target.join("usr/share/keyrings/t2-ubuntu-repo.gpg"));
}

fn install_bootloader(
    _plan: &InstallPlan,
    target: &Path,
    _root: &str,
    esp_mount: &Path,
    _medium: &str,
) -> Result<(), String> {
    let efi_dir = esp_mount.join("EFI/BOOT");
    fs::create_dir_all(&efi_dir).map_err(|e| format!("EFI dir: {e}"))?;
    let efi = efi_dir.join("BOOTX64.EFI");
    // The loader carries only the stub (configure_target): the real menu is
    // /boot/grub/grub.cfg on the root filesystem, kept by liteaura-update-boot.
    let stub = target.join("boot/grub/liteaura-efi.cfg");
    run(
        "grub-mkstandalone",
        &[
            "-O",
            "x86_64-efi",
            "--modules=part_gpt part_msdos ext2 search search_fs_uuid search_fs_file configfile linux normal",
            "-o",
            efi.to_str().ok_or("EFI path")?,
            &format!("boot/grub/grub.cfg={}", stub.display()),
        ],
    )?;
    if !efi.exists() {
        return Err("grub-mkstandalone did not create BOOTX64.EFI".into());
    }
    Ok(())
}

/// The installed session script, derived from the live one: no installer, and
/// the AI tier chosen in the wizard. The script turns the tier into the
/// server's switches (AI on/off, speech engines started/installed).
/// `/etc/default/liteaura`: the AI tier chosen in the installer, as the
/// variables the session and the per-user server read. Booleans are spelled
/// true/false: the server parses them as Rust bools.
fn machine_defaults(plan: &InstallPlan) -> String {
    let tier = match plan.ai_tier.as_str() {
        AI_TIER_OFF | AI_TIER_NONE | AI_TIER_VOICE | AI_TIER_LOCAL => plan.ai_tier.as_str(),
        _ => AI_TIER_VOICE,
    };
    let (default, voice_start, voice_install) = match tier {
        AI_TIER_OFF => ("off", false, false),
        AI_TIER_NONE => ("on", false, false),
        _ => ("on", true, true),
    };
    format!(
        "# Written by the LiteAura installer. Read by every account's session\n\
         # (shiny-session) and server (systemd user unit shiny.service).\n\
         LITEAURA_AI_TIER={tier}\n\
         LITEAURA_AI_DEFAULT={default}\n\
         AUTO_START_WHISPER={voice_start}\n\
         AUTO_START_SUPERTONIC={voice_start}\n\
         WHISPER_AUTO_INSTALL={voice_install}\n\
         SUPERTONIC_AUTO_INSTALL={voice_install}\n"
    )
}

/// Display managers the live image may carry; none of them may claim the seat.
const DISPLAY_MANAGERS: &[&str] = &["lightdm", "gdm", "gdm3", "sddm", "lxdm", "xdm", "slim", "nodm", "greetd", "ly"];

/// Put LiteAura's login screen on the target's seat, the same result as
/// `liteaura-seat enable` but written as plain symlinks (the target is not
/// booted, and its systemd must not run here): the password helper's socket
/// and the login screen enabled, every display manager off, the machine
/// booting to `multi-user.target`.
fn enable_seat(target: &Path) -> Result<(), String> {
    let units = target.join("etc/systemd/system");
    let _ = fs::remove_file(units.join("display-manager.service"));
    for dm in DISPLAY_MANAGERS {
        for wants in ["multi-user.target.wants", "graphical.target.wants"] {
            let _ = fs::remove_file(units.join(wants).join(format!("{dm}.service")));
        }
    }
    let link = |unit: &str, wants: &str| -> Result<(), String> {
        let dir = units.join(wants);
        fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        let path = dir.join(unit);
        let _ = fs::remove_file(&path);
        std::os::unix::fs::symlink(format!("/usr/lib/systemd/system/{unit}"), &path)
            .map_err(|e| format!("enable {unit}: {e}"))
    };
    link("shiny-auth.socket", "sockets.target.wants")?;
    link("shiny-greeter.service", "multi-user.target.wants")?;
    let default = units.join("default.target");
    let _ = fs::remove_file(&default);
    std::os::unix::fs::symlink("/lib/systemd/system/multi-user.target", &default)
        .map_err(|e| format!("default target: {e}"))?;
    Ok(())
}

/// "No slop" installs no AI at all: the Ollama runtime (binary, ~5 GB of
/// GPU/CPU libraries, its service) is not copied to the target. Ollama can be
/// installed again later from Updates.
const OLLAMA_PATHS: &[&str] = &[
    "usr/local/bin/ollama",
    "usr/local/lib/ollama",
    "var/lib/ollama",
    "etc/systemd/system/ollama.service",
];

fn remove_ai_payload(plan: &InstallPlan, target: &Path) {
    if plan.ai_tier != AI_TIER_OFF {
        return;
    }
    for path in OLLAMA_PATHS {
        let path = target.join(path);
        if path.is_dir() {
            let _ = fs::remove_dir_all(&path);
        } else {
            let _ = fs::remove_file(&path);
        }
    }
}

fn kernel_version(target: &Path, t2: bool) -> Result<String, String> {
    let mut versions = Vec::new();
    for entry in fs::read_dir(target.join("lib/modules")).map_err(|e| format!("modules: {e}"))?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if t2 == name.contains("-t2") {
            versions.push(name);
        }
    }
    versions.sort();
    versions.pop().ok_or_else(|| "no matching kernel modules found".into())
}

fn mount_chroot(target: &Path) -> Result<(), String> {
    let target = target.to_str().ok_or("target path")?;
    for (source, dest, fstype) in [
        ("proc", "/proc", "proc"),
        ("sysfs", "/sys", "sysfs"),
    ] {
        let dest = format!("{target}{dest}");
        fs::create_dir_all(&dest).ok();
        let _ = Command::new("mount").args(["-t", fstype, source, &dest]).output();
    }
    let dev = format!("{target}/dev");
    fs::create_dir_all(&dev).ok();
    let _ = Command::new("mount").args(["--bind", "/dev", &dev]).output();
    Ok(())
}

fn unmount_chroot(target: &Path) {
    let target = target.to_str().unwrap_or("");
    for path in ["/dev", "/sys", "/proc"] {
        let mount = format!("{target}{path}");
        let _ = Command::new("umount").arg(&mount).output();
        let _ = Command::new("umount").args(["-l"]).arg(&mount).output();
    }
}

fn uuid_of(path: &str) -> Result<String, String> {
    let out = run("blkid", &["-s", "UUID", "-o", "value", path])?;
    if out.trim().is_empty() {
        Err(format!("no UUID for {path}"))
    } else {
        Ok(out.trim().to_string())
    }
}

fn write(target: &Path, relative: &str, text: &str) -> Result<(), String> {
    let path = target.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

fn install_file(source: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    fs::copy(source, dest).map_err(|e| format!("copy {}: {e}", dest.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiny_installer_protocol::FreeExtent;

    fn manual_plan(extent: Option<FreeExtent>) -> InstallPlan {
        let mut plan = InstallPlan::default();
        plan.mode = MODE_MANUAL.into();
        plan.layout.free_extent = extent;
        plan
    }

    #[test]
    fn the_boot_menu_is_read_from_the_root_filesystem() {
        let stub = efi_stub_cfg("abcd-1234");
        assert!(stub.contains("search --no-floppy --fs-uuid --set=root abcd-1234"));
        assert!(stub.contains("configfile ($root)/boot/grub/grub.cfg"));
        assert!(!stub.contains("vmlinuz"), "the stub must not pin a kernel");
    }

    #[test]
    fn boot_conf_names_the_flavor_the_updater_follows() {
        let t2 = boot_conf("u-1", true);
        assert!(t2.contains("ROOT_UUID=u-1\n") && t2.contains("FLAVOR=t2\n"), "{t2}");
        assert!(boot_conf("u-1", false).contains("FLAVOR=generic\n"));
        assert!(t2.contains(&format!("CMDLINE=\"{KERNEL_CMDLINE}\"")));
    }

    #[test]
    fn only_apple_wireless_firmware_is_carried_over() {
        assert!(is_apple_wireless_firmware("brcmfmac4364b3-pcie.apple,borneo.bin"));
        assert!(is_apple_wireless_firmware("brcmbt4377b3-apple,formosa.ptb"));
        assert!(is_apple_wireless_firmware("brcmfmac4364b2-pcie.txcap_blob"));
        assert!(!is_apple_wireless_firmware("brcmfmac43430-sdio.bin"));
        assert!(!is_apple_wireless_firmware("cypress"));
    }

    #[test]
    fn the_chosen_extent_drives_the_start_and_end() {
        let plan = manual_plan(Some(FreeExtent {
            start_bytes: 1024 * MIB,
            size_bytes: 2048 * MIB,
        }));
        assert_eq!(free_space_start(&plan).unwrap(), 1024);
        assert_eq!(free_extent_end_mib(&plan).unwrap(), 3072);
    }

    #[test]
    fn the_ai_tier_becomes_the_machine_default() {
        let mut plan = manual_plan(None);
        for tier in [AI_TIER_OFF, AI_TIER_NONE, AI_TIER_VOICE, AI_TIER_LOCAL] {
            plan.ai_tier = tier.into();
            let text = machine_defaults(&plan);
            assert!(text.contains(&format!("\nLITEAURA_AI_TIER={tier}\n")), "{text}");
        }
        plan.ai_tier = AI_TIER_OFF.into();
        let off = machine_defaults(&plan);
        assert!(off.contains("LITEAURA_AI_DEFAULT=off\n") && off.contains("AUTO_START_WHISPER=false\n"));
        plan.ai_tier = AI_TIER_NONE.into();
        let none = machine_defaults(&plan);
        assert!(none.contains("LITEAURA_AI_DEFAULT=on\n") && none.contains("AUTO_START_SUPERTONIC=false\n"));
        plan.ai_tier = AI_TIER_LOCAL.into();
        assert!(machine_defaults(&plan).contains("WHISPER_AUTO_INSTALL=true\n"));
        plan.ai_tier = "bogus".into();
        assert!(machine_defaults(&plan).contains("LITEAURA_AI_TIER=voice\n"));
        // Every line is KEY=VALUE or a comment: systemd's EnvironmentFile reads it.
        for line in machine_defaults(&plan).lines() {
            assert!(line.starts_with('#') || line.split_once('=').is_some_and(|(k, _)| !k.contains(' ')), "{line}");
        }
    }

    #[test]
    fn the_target_signs_in_on_the_liteaura_login_screen() {
        let dir = std::env::temp_dir().join(format!("liteaura-seat-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let units = dir.join("etc/systemd/system");
        fs::create_dir_all(units.join("multi-user.target.wants")).unwrap();
        std::os::unix::fs::symlink("/lib/systemd/system/lightdm.service", units.join("display-manager.service")).unwrap();
        std::os::unix::fs::symlink("/lib/systemd/system/lightdm.service", units.join("multi-user.target.wants/lightdm.service")).unwrap();
        std::os::unix::fs::symlink("/lib/systemd/system/graphical.target", units.join("default.target")).unwrap();

        enable_seat(&dir).unwrap();
        assert!(fs::symlink_metadata(units.join("display-manager.service")).is_err(), "no display manager");
        assert!(fs::symlink_metadata(units.join("multi-user.target.wants/lightdm.service")).is_err());
        assert_eq!(
            fs::read_link(units.join("multi-user.target.wants/shiny-greeter.service")).unwrap(),
            Path::new("/usr/lib/systemd/system/shiny-greeter.service")
        );
        assert_eq!(
            fs::read_link(units.join("sockets.target.wants/shiny-auth.socket")).unwrap(),
            Path::new("/usr/lib/systemd/system/shiny-auth.socket")
        );
        assert_eq!(fs::read_link(units.join("default.target")).unwrap(), Path::new("/lib/systemd/system/multi-user.target"));
        // Running it twice is harmless.
        enable_seat(&dir).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_slop_leaves_no_ollama_on_the_target() {
        let dir = std::env::temp_dir().join(format!("liteaura-ai-payload-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("usr/local/lib/ollama/vulkan")).unwrap();
        fs::create_dir_all(dir.join("usr/local/bin")).unwrap();
        fs::write(dir.join("usr/local/bin/ollama"), "bin").unwrap();
        let mut plan = manual_plan(None);
        plan.ai_tier = AI_TIER_NONE.into();
        remove_ai_payload(&plan, &dir);
        assert!(dir.join("usr/local/bin/ollama").exists(), "text-only keeps Ollama");
        plan.ai_tier = AI_TIER_OFF.into();
        remove_ai_payload(&plan, &dir);
        assert!(!dir.join("usr/local/bin/ollama").exists());
        assert!(!dir.join("usr/local/lib/ollama").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_extent_is_refused() {
        let plan = manual_plan(None);
        assert!(free_space_start(&plan).unwrap_err().contains("chosen free space"));
        assert!(free_extent_end_mib(&plan).unwrap_err().contains("chosen free space"));
    }
}
