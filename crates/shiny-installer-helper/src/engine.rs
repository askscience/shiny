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
    InstallPlan, PartitionSpec, MODE_ERASE, MODE_EXISTING, MODE_MANUAL,
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

    let source_squash = std::env::var("PEAKD_ENGINE_SQUASHFS")
        .unwrap_or_else(|_| "/run/live/medium/live/filesystem.squashfs".into());
    let source_override = std::env::var("PEAKD_ENGINE_SOURCE").ok();
    let medium = std::env::var("PEAKD_ENGINE_MEDIUM")
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

    // The live session becomes a normal target session: no breakthrough
    // installer, no seed DB, no live-only services.
    let live_session = target.join("usr/local/bin/peakd-live-session");
    let target_session = target.join("usr/local/bin/peakd-session");
    if let Ok(text) = fs::read_to_string(&live_session) {
        let text = text.replace("export SHINY_LIVE_INSTALLER=1", "export SHINY_LIVE_INSTALLER=0");
        fs::write(&target_session, text).map_err(|e| format!("target session: {e}"))?;
        set_exec(&target_session)?;
    }
    let _ = fs::remove_file(&live_session);
    let desktop = target.join("usr/share/xsessions/peakd.desktop");
    if let Ok(text) = fs::read_to_string(&desktop) {
        fs::write(&desktop, text.replace("peakd-live-session", "peakd-session"))
            .map_err(|e| format!("session desktop: {e}"))?;
    }
    let lightdm = target.join("etc/lightdm/lightdm.conf.d/99-peakd.conf");
    if let Ok(text) = fs::read_to_string(&lightdm) {
        let text = text
            .replace("autologin-user=user", &format!("autologin-user={}", plan.username))
            .replace("autologin-user-timeout=0", "autologin-user-timeout=0");
        fs::write(&lightdm, text).map_err(|e| format!("lightdm: {e}"))?;
    }

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
    let display_manager = target.join("etc/systemd/system/display-manager.service");
    let lightdm = target.join("lib/systemd/system/lightdm.service");
    if lightdm.exists() {
        let _ = std::os::unix::fs::symlink("/lib/systemd/system/lightdm.service", display_manager);
        let wants = target.join("etc/systemd/system/multi-user.target.wants");
        let _ = fs::create_dir_all(&wants);
        let _ = std::os::unix::fs::symlink(
            "/lib/systemd/system/lightdm.service",
            wants.join("lightdm.service"),
        );
    }
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
        "GRUB_DEFAULT=0\nGRUB_TIMEOUT=5\nGRUB_DISTRIBUTOR=\"PEAK'D!\"\nGRUB_CMDLINE_LINUX=\"quiet rw console=tty0 console=ttyS0,115200\"\nGRUB_CMDLINE_LINUX_DEFAULT=\"quiet rw\"\n",
    )?;

    let version = kernel_version(target, plan.is_t2())?;
    let kernel_source = Path::new(medium).join(if plan.is_t2() { "vmlinuz-t2" } else { "vmlinuz" });
    if !kernel_source.exists() {
        return Err(format!("kernel image missing: {}", kernel_source.display()));
    }
    install_file(&kernel_source, &target.join(format!("boot/vmlinuz-{version}")))?;

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

    let grub_cfg = format!(
        "set timeout=5\nset default=0\ninsmod part_gpt\ninsmod ext2\ninsmod search_fs_uuid\n\
         search --no-floppy --fs-uuid --set=root {root_uuid}\n\
         menuentry \"PEAK'D!\" {{\n linux /boot/vmlinuz-{version} root=UUID={root_uuid} rw quiet console=tty0 console=ttyS0,115200\n initrd /boot/initrd.img-{version}\n}}\n"
    );
    write(target, "boot/grub/grub.cfg", &grub_cfg)?;
    mount_chroot(target)?;
    Ok(())
}

fn install_bootloader(
    plan: &InstallPlan,
    target: &Path,
    _root: &str,
    esp_mount: &Path,
    _medium: &str,
) -> Result<(), String> {
    let efi_dir = esp_mount.join("EFI/BOOT");
    fs::create_dir_all(&efi_dir).map_err(|e| format!("EFI dir: {e}"))?;
    let efi = efi_dir.join("BOOTX64.EFI");
    let grub_cfg = target.join("boot/grub/grub.cfg");
    run(
        "grub-mkstandalone",
        &[
            "-O",
            "x86_64-efi",
            "--modules=part_gpt part_msdos ext2 search_fs_uuid search_fs_file linux normal",
            "-o",
            efi.to_str().ok_or("EFI path")?,
            &format!("boot/grub/grub.cfg={}", grub_cfg.display()),
        ],
    )?;
    if !efi.exists() {
        return Err("grub-mkstandalone did not create BOOTX64.EFI".into());
    }
    Ok(())
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

fn set_exec(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path).map_err(|e| format!("stat {}: {e}", path.display()))?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).map_err(|e| format!("chmod {}: {e}", path.display()))
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
    fn the_chosen_extent_drives_the_start_and_end() {
        let plan = manual_plan(Some(FreeExtent {
            start_bytes: 1024 * MIB,
            size_bytes: 2048 * MIB,
        }));
        assert_eq!(free_space_start(&plan).unwrap(), 1024);
        assert_eq!(free_extent_end_mib(&plan).unwrap(), 3072);
    }

    #[test]
    fn a_missing_extent_is_refused() {
        let plan = manual_plan(None);
        assert!(free_space_start(&plan).unwrap_err().contains("chosen free space"));
        assert!(free_extent_end_mib(&plan).unwrap_err().contains("chosen free space"));
    }
}
