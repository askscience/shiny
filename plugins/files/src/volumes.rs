//! Removable drives — USB sticks, SD cards, external disks.
//!
//! Two halves:
//!
//! * **Where a drive's files are** (sync, sysfs + `/proc/self/mountinfo`): a
//!   mounted removable filesystem is browsable under the virtual path
//!   `@volumes/<kernel name>/…` (`@volumes/sdb1/Photos`). `fs_util::resolve`
//!   maps that onto the mount point and keeps the same escape checks it uses
//!   for the home folder. Only *removable* devices qualify (USB transport,
//!   the kernel's removable flag, or an SD card), so the window can never be
//!   pointed at a system disk.
//! * **Mounting** (async, UDisks2 over the system D-Bus): list the drives,
//!   mount / unmount / eject them as the signed-in user (polkit lets the
//!   active local session do that without a password), and **automount** a
//!   drive the moment it is plugged in. Without udisks2 the list still comes
//!   from `lsblk`, already-mounted drives are still browsable, and the window
//!   says mounting needs udisks2.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

/// Path prefix of the virtual drives root.
pub const PREFIX: &str = "@volumes";

/* ── kernel view: which devices are removable, where they are mounted ── */

/// A kernel block device name we accept in a path (`sdb1`, `mmcblk0p1`).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn read_trim(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Whether `/sys/class/block/<name>` is a removable device (or a partition of
/// one): on a USB bus, flagged removable by the kernel, or an SD card.
pub fn is_removable(name: &str) -> bool {
    if !valid_id(name) {
        return false;
    }
    let class = PathBuf::from("/sys/class/block").join(name);
    let Ok(real) = std::fs::canonicalize(&class) else {
        return false;
    };
    let real_str = real.to_string_lossy();
    if real_str.contains("/usb") {
        return true;
    }
    // The whole disk: the partition's parent directory in sysfs.
    let disk = if class.join("partition").exists() {
        real.parent().map(Path::to_path_buf).unwrap_or(real.clone())
    } else {
        real.clone()
    };
    if read_trim(disk.join("removable")).as_deref() == Some("1") {
        return true;
    }
    read_trim(disk.join("device/type")).as_deref() == Some("SD")
}

/// `\040`-style escapes in mountinfo fields.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            if let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 4])
                .ok()
                .and_then(|oct| u8::from_str_radix(oct, 8).ok())
            {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `(source, mount point)` pairs from a mountinfo document.
fn parse_mountinfo(text: &str) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for line in text.lines() {
        // id parent maj:min root mountpoint opts [optional…] - fstype source superopts
        let Some((left, right)) = line.split_once(" - ") else { continue };
        let mount_point = left.split(' ').nth(4).map(unescape);
        let source = right.split(' ').nth(1).map(unescape);
        if let (Some(mp), Some(src)) = (mount_point, source) {
            out.push((src, PathBuf::from(mp)));
        }
    }
    out
}

fn mounts_cache() -> &'static Mutex<Option<(Instant, Vec<(String, PathBuf)>)>> {
    static CACHE: Mutex<Option<(Instant, Vec<(String, PathBuf)>)>> = Mutex::new(None);
    &CACHE
}

/// Mounted removable filesystems as `(kernel name, mount point)`. Cached for a
/// second: a folder listing asks once per entry.
pub fn removable_mounts() -> Vec<(String, PathBuf)> {
    let mut cache = mounts_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, list)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(1) {
            return list.clone();
        }
    }
    let text = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let mut list = Vec::new();
    for (source, mp) in parse_mountinfo(&text) {
        let Some(name) = source.strip_prefix("/dev/") else { continue };
        if mp == Path::new("/") || list.iter().any(|(n, _): &(String, PathBuf)| n == name) {
            continue;
        }
        if is_removable(name) {
            list.push((name.to_string(), mp));
        }
    }
    *cache = Some((Instant::now(), list.clone()));
    list
}

/// Forget the cached mount table (after a mount or an unmount).
pub fn invalidate() {
    *mounts_cache().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The mount point of removable device `id`, when it is mounted.
pub fn mount_root(id: &str) -> Option<PathBuf> {
    if !valid_id(id) {
        return None;
    }
    removable_mounts().into_iter().find(|(n, _)| n == id).map(|(_, mp)| mp)
}

/// The drive a path lies on: `(id, mount point)`, the longest match winning.
pub fn root_containing(path: &Path) -> Option<(String, PathBuf)> {
    removable_mounts()
        .into_iter()
        .filter(|(_, mp)| path.starts_with(mp))
        .max_by_key(|(_, mp)| mp.as_os_str().len())
}

/// Split `@volumes/<id>/rest` into `(id, rest)`. `None` for any other path.
pub fn split(rel: &str) -> Option<(String, String)> {
    let rel = rel.trim();
    let rel = rel.strip_prefix("./").unwrap_or(rel);
    let rest = rel.strip_prefix(PREFIX)?;
    if !(rest.is_empty() || rest.starts_with('/')) {
        return None;
    }
    let rest = rest.trim_start_matches('/');
    let (id, tail) = rest.split_once('/').unwrap_or((rest, ""));
    Some((id.to_string(), tail.to_string()))
}

/// Whether a request path points at a drive.
pub fn is_volume_path(rel: &str) -> bool {
    split(rel).is_some()
}

/* ── settings: automount on/off ── */

fn settings_file() -> PathBuf {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    root.join("liteaura").join("files.json")
}

/// Whether a drive is mounted as soon as it is plugged in (default on).
pub fn automount_enabled() -> bool {
    std::fs::read_to_string(settings_file())
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| v.get("automount").and_then(Value::as_bool))
        .unwrap_or(true)
}

pub fn set_automount(on: bool) -> Result<(), String> {
    let path = settings_file();
    let mut doc = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    doc["automount"] = json!(on);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&doc).unwrap_or_default()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/* ── the drive list ── */

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Volume {
    /// Kernel name (`sdb1`): the `@volumes/<id>` path segment.
    pub id: String,
    /// What to call it: the filesystem label, else the drive model, else the size.
    pub label: String,
    pub fs_type: String,
    pub size: u64,
    /// The drive it is on (`SanDisk Ultra`), for grouping and the eject button.
    pub drive: String,
    pub mounted: bool,
    /// `@volumes/<id>` when mounted.
    pub path: Option<String>,
    pub can_eject: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct VolumeList {
    pub volumes: Vec<Volume>,
    /// udisks2 is reachable: mount / unmount / eject work.
    pub can_mount: bool,
    pub automount: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[i]) }
}

fn label_for(label: &str, drive: &str, size: u64) -> String {
    let label = label.trim();
    if !label.is_empty() {
        return label.to_string();
    }
    if size > 0 {
        return format!("{} Volume", human_size(size));
    }
    if !drive.trim().is_empty() {
        return drive.trim().to_string();
    }
    "Drive".into()
}

fn finish(mut volumes: Vec<Volume>) -> Vec<Volume> {
    let mounts = removable_mounts();
    for v in &mut volumes {
        if mounts.iter().any(|(n, _)| *n == v.id) {
            v.mounted = true;
            v.path = Some(format!("{PREFIX}/{}", v.id));
        }
    }
    volumes.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()).then(a.id.cmp(&b.id)));
    volumes
}

/// Every removable filesystem, mounted or not.
pub async fn list() -> VolumeList {
    let automount = automount_enabled();
    match udisks::list().await {
        Ok(volumes) => VolumeList { volumes: finish(volumes), can_mount: true, automount, reason: None },
        Err(err) => VolumeList {
            volumes: finish(lsblk_list().await),
            can_mount: false,
            automount,
            reason: Some(err),
        },
    }
}

/// The fallback list: `lsblk`, filtered to removable devices with a filesystem.
async fn lsblk_list() -> Vec<Volume> {
    let out = tokio::process::Command::new("lsblk")
        .args(["-J", "-b", "-o", "NAME,LABEL,SIZE,FSTYPE,MODEL,VENDOR"])
        .output()
        .await;
    let Ok(out) = out else { return Vec::new() };
    let Ok(doc) = serde_json::from_slice::<Value>(&out.stdout) else { return Vec::new() };
    let mut volumes = Vec::new();
    fn walk(node: &Value, drive: &str, out: &mut Vec<Volume>) {
        let s = |k: &str| node.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
        let name = s("name");
        let model = [s("vendor"), s("model")].join(" ").trim().to_string();
        let drive = if drive.is_empty() { model } else { drive.to_string() };
        let size = node.get("size").and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok())).unwrap_or(0);
        let fs_type = s("fstype");
        if !fs_type.is_empty() && fs_type != "swap" && is_removable(&name) {
            out.push(Volume {
                label: label_for(&s("label"), &drive, size),
                id: name,
                fs_type,
                size,
                drive: drive.clone(),
                mounted: false,
                path: None,
                can_eject: false,
            });
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children {
                walk(child, &drive, out);
            }
        }
    }
    for dev in doc.get("blockdevices").and_then(Value::as_array).into_iter().flatten() {
        walk(dev, "", &mut volumes);
    }
    volumes
}

pub async fn mount(id: &str) -> Result<String, String> {
    if !valid_id(id) || !is_removable(id) {
        return Err("not a removable drive".into());
    }
    let result = udisks::mount(id).await;
    invalidate();
    result.map(|_| format!("{PREFIX}/{id}"))
}

pub async fn unmount(id: &str) -> Result<(), String> {
    if !valid_id(id) || !is_removable(id) {
        return Err("not a removable drive".into());
    }
    let result = udisks::unmount(id).await;
    invalidate();
    result
}

/// Unmount every filesystem on the drive `id` belongs to, then power it off
/// so it is safe to pull out.
pub async fn eject(id: &str) -> Result<(), String> {
    if !valid_id(id) || !is_removable(id) {
        return Err("not a removable drive".into());
    }
    let result = udisks::eject(id).await;
    invalidate();
    result
}

/// Mount drives as they are plugged in, for as long as the plugin runs.
pub async fn watch() {
    udisks::watch().await;
}

#[cfg(target_os = "linux")]
mod udisks {
    use super::*;
    use futures::StreamExt;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue};

    const DEST: &str = "org.freedesktop.UDisks2";
    const ROOT: &str = "/org/freedesktop/UDisks2";
    const BLOCK: &str = "org.freedesktop.UDisks2.Block";
    const FS: &str = "org.freedesktop.UDisks2.Filesystem";
    const DRIVE: &str = "org.freedesktop.UDisks2.Drive";

    type Props = HashMap<String, OwnedValue>;
    type Objects = HashMap<OwnedObjectPath, HashMap<zbus::names::OwnedInterfaceName, Props>>;

    fn conn_slot() -> &'static tokio::sync::Mutex<Option<zbus::Connection>> {
        static SLOT: tokio::sync::Mutex<Option<zbus::Connection>> = tokio::sync::Mutex::const_new(None);
        &SLOT
    }

    async fn connection() -> Result<zbus::Connection, String> {
        let mut slot = conn_slot().lock().await;
        if let Some(c) = slot.as_ref() {
            return Ok(c.clone());
        }
        let c = zbus::Connection::system().await.map_err(|e| format!("system bus unavailable: {e}"))?;
        *slot = Some(c.clone());
        Ok(c)
    }

    async fn objects(conn: &zbus::Connection) -> Result<Objects, String> {
        let proxy = zbus::fdo::ObjectManagerProxy::new(conn, DEST, ROOT)
            .await
            .map_err(|e| e.to_string())?;
        proxy.get_managed_objects().await.map_err(|e| {
            let text = e.to_string();
            if text.contains("ServiceUnknown") || text.contains("was not provided") {
                "udisks2 is not installed".to_string()
            } else {
                text
            }
        })
    }

    fn iface<'a>(ifaces: &'a HashMap<zbus::names::OwnedInterfaceName, Props>, name: &str) -> Option<&'a Props> {
        ifaces.iter().find(|(k, _)| k.as_str() == name).map(|(_, v)| v)
    }

    fn p_bool(p: &Props, k: &str) -> bool {
        p.get(k).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
    }

    fn p_u64(p: &Props, k: &str) -> u64 {
        p.get(k).and_then(|v| u64::try_from(v).ok()).unwrap_or(0)
    }

    fn p_str(p: &Props, k: &str) -> String {
        p.get(k)
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| String::try_from(v).ok())
            .unwrap_or_default()
    }

    fn p_path(p: &Props, k: &str) -> Option<OwnedObjectPath> {
        p.get(k)
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| OwnedObjectPath::try_from(v).ok())
            .filter(|o| o.as_str() != "/")
    }

    /// One udisks block object as a [`Volume`] candidate, plus its drive path.
    struct Candidate {
        volume: Volume,
        block_path: OwnedObjectPath,
        drive_path: Option<OwnedObjectPath>,
        mounted: bool,
    }

    fn candidates(objs: &Objects) -> Vec<Candidate> {
        let mut out = Vec::new();
        for (path, ifaces) in objs {
            let (Some(block), Some(fs)) = (iface(ifaces, BLOCK), iface(ifaces, FS)) else { continue };
            let Some(id) = path.as_str().rsplit('/').next().map(str::to_string) else { continue };
            if p_bool(block, "HintIgnore") || p_str(block, "IdUsage") != "filesystem" {
                continue;
            }
            let drive_path = p_path(block, "Drive");
            let drive = drive_path
                .as_ref()
                .and_then(|d| objs.get(d))
                .and_then(|i| iface(i, DRIVE));
            let removable = drive.is_some_and(|d| {
                p_bool(d, "Removable") || p_bool(d, "MediaRemovable") || matches!(p_str(d, "ConnectionBus").as_str(), "usb" | "sdio")
            });
            if !removable && (p_bool(block, "HintSystem") || !is_removable(&id)) {
                continue;
            }
            let drive_name = drive
                .map(|d| [p_str(d, "Vendor"), p_str(d, "Model")].join(" ").trim().to_string())
                .unwrap_or_default();
            let size = p_u64(block, "Size");
            let mounted = fs
                .get("MountPoints")
                .and_then(|v| v.try_clone().ok())
                .and_then(|v| Vec::<Vec<u8>>::try_from(v).ok())
                .is_some_and(|m| !m.is_empty());
            let can_eject = drive.is_some_and(|d| p_bool(d, "CanPowerOff") || p_bool(d, "Ejectable"));
            out.push(Candidate {
                volume: Volume {
                    label: label_for(&p_str(block, "IdLabel"), &drive_name, size),
                    id,
                    fs_type: p_str(block, "IdType"),
                    size,
                    drive: drive_name,
                    mounted: false,
                    path: None,
                    can_eject,
                },
                block_path: path.clone(),
                drive_path,
                mounted,
            });
        }
        out
    }

    pub async fn list() -> Result<Vec<Volume>, String> {
        let conn = connection().await?;
        let objs = objects(&conn).await?;
        Ok(candidates(&objs).into_iter().map(|c| c.volume).collect())
    }

    async fn find(conn: &zbus::Connection, id: &str) -> Result<(Candidate, Vec<Candidate>), String> {
        let objs = objects(conn).await?;
        let all = candidates(&objs);
        let me = all.iter().position(|c| c.volume.id == id).ok_or("drive not found")?;
        let mut all = all;
        let me = all.swap_remove(me);
        Ok((me, all))
    }

    fn opts() -> HashMap<&'static str, zbus::zvariant::Value<'static>> {
        HashMap::new()
    }

    fn friendly(err: zbus::Error) -> String {
        let text = err.to_string();
        if text.contains("NotAuthorized") {
            "not allowed to mount this drive".into()
        } else if text.contains("DeviceBusy") || text.contains("target is busy") {
            "the drive is in use".into()
        } else if text.contains("AlreadyMounted") {
            "already mounted".into()
        } else {
            text
        }
    }

    async fn call(conn: &zbus::Connection, path: &OwnedObjectPath, iface: &str, method: &str) -> Result<zbus::Message, String> {
        conn.call_method(Some(DEST), path.as_str(), Some(iface), method, &(opts(),))
            .await
            .map_err(friendly)
    }

    pub async fn mount(id: &str) -> Result<String, String> {
        let conn = connection().await?;
        let (me, _) = find(&conn, id).await?;
        if me.mounted {
            return Ok(String::new());
        }
        let reply = call(&conn, &me.block_path, FS, "Mount").await?;
        reply.body().deserialize::<String>().map_err(|e| e.to_string())
    }

    pub async fn unmount(id: &str) -> Result<(), String> {
        let conn = connection().await?;
        let (me, _) = find(&conn, id).await?;
        if !me.mounted {
            return Ok(());
        }
        call(&conn, &me.block_path, FS, "Unmount").await.map(|_| ())
    }

    pub async fn eject(id: &str) -> Result<(), String> {
        let conn = connection().await?;
        let (me, others) = find(&conn, id).await?;
        // Every mounted filesystem on the same drive goes first.
        for c in std::iter::once(&me).chain(others.iter().filter(|o| o.drive_path.is_some() && o.drive_path == me.drive_path)) {
            if c.mounted {
                call(&conn, &c.block_path, FS, "Unmount").await?;
            }
        }
        let Some(drive) = me.drive_path.as_ref() else { return Ok(()) };
        let objs = objects(&conn).await?;
        let props = objs.get(drive).and_then(|i| iface(i, DRIVE));
        let can_power_off = props.is_some_and(|d| p_bool(d, "CanPowerOff"));
        let ejectable = props.is_some_and(|d| p_bool(d, "Ejectable"));
        if can_power_off {
            call(&conn, drive, DRIVE, "PowerOff").await.map(|_| ())
        } else if ejectable {
            call(&conn, drive, DRIVE, "Eject").await.map(|_| ())
        } else {
            Ok(())
        }
    }

    /// Follow udisks' object manager: a filesystem that appears on a removable
    /// drive (a stick plugged in) is mounted when automount is on. Drives that
    /// were already there when the plugin started are left as they are, so an
    /// unmount the user made survives a restart.
    pub async fn watch() {
        let mut logged = false;
        loop {
            match follow().await {
                Ok(()) => {}
                Err(err) => {
                    if !logged {
                        tracing::info!("files: drive automount waiting for udisks2 ({err})");
                        logged = true;
                    }
                }
            }
            *conn_slot().lock().await = None;
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }

    async fn follow() -> Result<(), String> {
        let conn = connection().await?;
        let proxy = zbus::fdo::ObjectManagerProxy::new(&conn, DEST, ROOT)
            .await
            .map_err(|e| e.to_string())?;
        let mut added = proxy.receive_interfaces_added().await.map_err(|e| e.to_string())?;
        let mut known: HashSet<String> = candidates(&objects(&conn).await?)
            .into_iter()
            .map(|c| c.volume.id)
            .collect();
        tracing::info!("files: following udisks2 for removable drives");
        while let Some(signal) = added.next().await {
            let Ok(args) = signal.args() else { continue };
            if !args.interfaces_and_properties().keys().any(|k| k.as_str() == FS || k.as_str() == BLOCK) {
                continue;
            }
            // udisks fills a new block object in over a moment (probe, label).
            tokio::time::sleep(Duration::from_millis(600)).await;
            let Ok(objs) = objects(&conn).await else { continue };
            let current = candidates(&objs);
            let ids: HashSet<String> = current.iter().map(|c| c.volume.id.clone()).collect();
            for c in &current {
                if known.contains(&c.volume.id) || c.mounted || !automount_enabled() {
                    continue;
                }
                match call(&conn, &c.block_path, FS, "Mount").await {
                    Ok(_) => tracing::info!("files: mounted {} ({})", c.volume.label, c.volume.id),
                    Err(err) => tracing::warn!("files: could not mount {}: {err}", c.volume.id),
                }
                invalidate();
            }
            // A drive that went away and comes back counts as new again.
            known = ids;
        }
        Err("udisks2 went away".into())
    }
}

#[cfg(not(target_os = "linux"))]
mod udisks {
    use super::Volume;

    const NONE: &str = "removable drives are handled by the operating system";

    pub async fn list() -> Result<Vec<Volume>, String> {
        Err(NONE.into())
    }
    pub async fn mount(_id: &str) -> Result<String, String> {
        Err(NONE.into())
    }
    pub async fn unmount(_id: &str) -> Result<(), String> {
        Err(NONE.into())
    }
    pub async fn eject(_id: &str) -> Result<(), String> {
        Err(NONE.into())
    }
    pub async fn watch() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mountinfo_is_parsed_and_unescaped() {
        let text = "36 25 8:17 / /media/eev/My\\040Stick rw,nosuid shared:1 - vfat /dev/sdb1 rw\n\
                    22 1 259:2 / / rw - ext4 /dev/nvme0n1p2 rw\n";
        let m = parse_mountinfo(text);
        assert_eq!(m[0], ("/dev/sdb1".into(), PathBuf::from("/media/eev/My Stick")));
        assert_eq!(m[1].1, PathBuf::from("/"));
    }

    #[test]
    fn volume_paths_split() {
        assert_eq!(split("@volumes/sdb1/Photos/a.jpg"), Some(("sdb1".into(), "Photos/a.jpg".into())));
        assert_eq!(split("@volumes/sdb1"), Some(("sdb1".into(), String::new())));
        assert_eq!(split("./@volumes/sdb1/"), Some(("sdb1".into(), String::new())));
        assert_eq!(split("@volumesX/a"), None);
        assert_eq!(split("Documents/@volumes"), None);
        assert!(!valid_id("../sda"));
        assert!(valid_id("mmcblk0p1"));
    }

    #[test]
    fn labels_fall_back_to_size_then_model() {
        assert_eq!(label_for(" KEY ", "SanDisk", 1), "KEY");
        assert_eq!(label_for("", "SanDisk", 16_000_000_000), "16.0 GB Volume");
        assert_eq!(label_for("", "SanDisk", 0), "SanDisk");
    }
}

#[cfg(test)]
mod live {
    /// Read-only probe of this machine: `cargo test -p shiny-files-plugin -- --ignored volumes_live`.
    #[tokio::test]
    #[ignore]
    async fn volumes_live_list() {
        println!("{}", serde_json::to_string_pretty(&super::list().await).unwrap());
    }
}
