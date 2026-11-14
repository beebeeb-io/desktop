//! What Drive's settings read from the engine and the local state: the Bandwidth pane's limits and traffic, the Sync
//! pane's sizes, the folder tree and the Storage insights. Split from `drive_settings` (its module doc has the
//! rules); the names other modules use are re-exported from there.

use super::*;

/// The limits from the settings file, and the last 20 hours of traffic from `account`'s local state (not handed
/// out, or another account's: no traffic). `None`: the limits could not be read, or the account left.
pub(crate) fn read_bandwidth<E: DriveEngine>(engine: &E, account: &str) -> Option<(TransferLimits, Option<History>)> {
    if !still_for(engine, account) {
        return None;
    }
    let limits = engine.limits().ok()?;
    let now = chrono::Local::now();
    let since = now.timestamp() - TRAFFIC_HOURS * 3_600;
    let samples = engine
        .local_state(|db| db.get_bandwidth_history_since(since).ok())
        .ok()
        .flatten();
    Some((limits, samples.map(|samples| history_of(&samples, now))))
}

/// The last 20 hours in buckets of 20 minutes, oldest first, the last bucket ending at `now`. No sample at all:
/// the empty history ("nothing recorded yet").
pub(crate) fn history_of(samples: &[BandwidthSample], now: chrono::DateTime<chrono::Local>) -> History {
    use chrono::Timelike;
    if samples.is_empty() {
        return History::default();
    }
    let bucket = BUCKET_MINUTES * 60;
    let buckets = (TRAFFIC_HOURS * 3_600 / bucket) as usize;
    let start = now.timestamp() - TRAFFIC_HOURS * 3_600;
    let (mut up, mut down) = (vec![0u64; buckets], vec![0u64; buckets]);
    for sample in samples {
        let Ok(i) = usize::try_from((sample.sampled_at - start).div_euclid(bucket)) else {
            continue;
        };
        if let (Some(up), Some(down)) = (up.get_mut(i), down.get_mut(i)) {
            *up = up.saturating_add(sample.up_bytes);
            *down = down.saturating_add(sample.down_bytes);
        }
    }
    let total = |bytes: &[u64]| size_words(i64::try_from(bytes.iter().sum::<u64>()).unwrap_or(i64::MAX));
    let mb = |bytes: &[u64]| bytes.iter().map(|b| *b as f32 / 1e6).collect::<Vec<f32>>();
    let (up_mb, down_mb) = (mb(&up), mb(&down));
    let most = up_mb.iter().chain(&down_mb).copied().fold(0.0_f32, f32::max);
    let first = now - chrono::Duration::seconds(TRAFFIC_HOURS * 3_600);
    History {
        up_total: total(&up),
        down_total: total(&down),
        // The axis ends on the next hundred megabytes over the busiest bucket (at least one hundred).
        top_mb: ((most / 100.0).ceil() * 100.0).max(100.0),
        starts_at: first.hour() * 60 + first.minute(),
        bucket_minutes: BUCKET_MINUTES as u32,
        starts_days_ago: u32::try_from((now.date_naive() - first.date_naive()).num_days()).unwrap_or(0),
        up_mb,
        down_mb,
    }
}

/// A speed limit as the pane counts it (megabits per second) in the unit the engine's transfer paths work in (KiB
/// per second). No limit is 0. Never 0 for a limit that was asked for.
pub(crate) fn kib_per_s(mbps: Option<u32>) -> u64 {
    mbps.map_or(0, |mbps| (u64::from(mbps) * 125_000 / 1_024).max(1))
}

/// The stored speed limit in megabits per second, to the nearest (at least 1); 0 is no limit.
pub(crate) fn mbps(kib_per_s: u64) -> Option<u32> {
    (kib_per_s > 0).then(|| u32::try_from(((kib_per_s * 1_024 + 62_500) / 125_000).max(1)).unwrap_or(u32::MAX))
}

pub(crate) fn read_local<E: DriveEngine>(engine: &E, platform: &dyn Platform, account: &str, send: &dyn Fn(Answer)) {
    send(Answer::OpenAtLogin(platform.autostart_enabled().ok()));
    if !still_for(engine, account) {
        // Locked, signed out or another account: no folder name stays in memory.
        send(Answer::OnThisPc(None));
        send(Answer::Keep(Keep::Failed));
        return;
    }
    let read = engine.local_state(|db| -> Result<_, String> {
        let rows = rows(db)?;
        let (limit_bytes, status_on_files) = engine.local_settings()?;
        let downloaded = db.cache_bytes_by_effective_pin(false).map_err(|_| "cache")?;
        let summary = db.kept_summary().map_err(|_| "kept")?;
        let on_pc = OnThisPc {
            downloaded,
            kept_items: summary.items as usize,
            kept_bytes: summary.bytes,
            limit_bytes,
            status_on_files,
        };
        Ok(Some((on_pc, keep_of(&rows))))
    });
    // Task 2488: a local state that is not this account's reads as nothing; so does one read across a lock or a
    // sign-out. Anything else that is not handed out could not be read.
    let read = match read {
        Ok(read) => read,
        Err(refused) if refused.not_this_accounts() || refused == LocalStateRefused::SessionChanged => Ok(None),
        Err(_) => Err("not handed out".to_string()),
    };
    match read {
        Ok(Some((on_pc, keep))) => {
            send(Answer::OnThisPc(Some(on_pc)));
            send(Answer::Keep(keep));
        }
        Ok(None) => {
            // Not this account's state (task 2488): nothing.
            send(Answer::OnThisPc(None));
            send(Answer::Keep(Keep::Failed));
        }
        Err(_) => {
            log("drive settings: the local state could not be read");
            send(Answer::Keep(Keep::Failed));
        }
    }
}

/// The live rows a page may list (never a device's manifest in Backups), each with whether it is kept.
pub(crate) fn rows(db: &StateDb) -> Result<Vec<(FileEntry, bool, DriveItem)>, String> {
    let backups = BackupsTree::read(db);
    let mut rows = Vec::new();
    for entry in model::live_rows(db).map_err(|_| "rows")? {
        if backups.is_hidden(&entry.path) {
            continue;
        }
        let contract = db.get_file_contract_state(&entry.file_id).map_err(|_| "contract")?;
        let kept = contract
            .as_ref()
            .is_some_and(|c| c.effective_pin_state() == PinState::Pinned);
        let item = model::item_from(&entry, contract.as_ref(), None);
        rows.push((entry, kept, item));
    }
    Ok(rows)
}

/// The files among `rows` (no folders).
pub(crate) fn files_of(rows: &[(FileEntry, bool, DriveItem)]) -> Vec<&FileEntry> {
    rows.iter().map(|(e, _, _)| e).filter(|e| !e.is_dir()).collect()
}

/// Storage insights: the account's use from Beebeeb, the rest from this PC's file list.
pub(crate) fn insights<E: DriveEngine>(engine: &E, account: &str) -> (Insights, Option<(i64, i64)>) {
    if !still_for(engine, account) {
        return (Insights::Failed(INSIGHTS_LOCKED.into()), None);
    }
    let usage = match engine.storage_info() {
        Ok(usage) => usage,
        Err(kind) => {
            let words = if kind == "offline" {
                INSIGHTS_OFFLINE
            } else {
                INSIGHTS_FAILED
            };
            return (Insights::Failed(words.into()), None);
        }
    };
    // Task 2488: another account's local state reads as nothing for insights.
    let read = engine.local_state(|db| rows(db).map(|rows| insights_of(&files_of(&rows), usage)));
    match read {
        Ok(Ok(insights)) => (insights, Some(usage)),
        // Not this account's state: show usage only, no breakdown.
        Err(refused) if refused.not_this_accounts() => (insights_of(&[], usage), Some(usage)),
        Ok(Err(_)) | Err(_) => {
            log("drive settings: the file list could not be read for insights");
            (Insights::Failed(INSIGHTS_FAILED.into()), Some(usage))
        }
    }
}

/// What a file counts as on the page.
pub(crate) fn kind_name(path: &str) -> &'static str {
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "heic" | "heif" | "webp" | "tif" | "tiff" | "bmp" | "raw" | "dng" | "cr2"
        | "cr3" | "nef" | "arw" | "raf" | "orf" | "braw" | "mov" | "mp4" | "m4v" | "avi" | "mkv" | "webm" | "wmv"
        | "mts" | "mp3" | "m4a" | "wav" | "flac" | "aac" | "ogg" | "opus" | "aiff" => "Media",
        "pdf" | "doc" | "docx" | "odt" | "rtf" | "txt" | "md" | "pages" | "xls" | "xlsx" | "xlsm" | "csv" | "ods"
        | "numbers" | "ppt" | "pptx" | "key" | "odp" | "epub" => "Documents",
        _ => "Other",
    }
}

/// The page for these files and this use. No files and nothing used: the empty state.
pub(crate) fn insights_of(files: &[&FileEntry], (used, quota): (i64, i64)) -> Insights {
    if files.is_empty() && used == 0 {
        return Insights::Empty;
    }
    let mut kinds: Vec<(&'static str, i64, usize)> = Vec::new();
    for file in files {
        let name = kind_name(&file.path);
        match kinds.iter_mut().find(|(k, _, _)| *k == name) {
            Some(kind) => {
                kind.1 += file.size_bytes.max(0);
                kind.2 += 1;
            }
            None => kinds.push((name, file.size_bytes.max(0), 1)),
        }
    }
    kinds.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let largest_kind = kinds.first().map_or(0, |k| k.1).max(1) as f32;
    let mut largest: Vec<&FileEntry> = files.to_vec();
    largest.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then(a.path.cmp(&b.path)));
    largest.truncate(LARGEST_TOP);
    let more = (largest.len() > LARGEST_SHOWN).then(|| {
        (
            format!("And {} more, down to ", largest.len() - LARGEST_SHOWN),
            size_words(largest.last().map_or(0, |f| f.size_bytes)),
        )
    });
    let total_bytes: i64 = files.iter().map(|f| f.size_bytes.max(0)).sum();
    let share = if quota > 0 {
        (used as f64 / quota as f64).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    Insights::Ready(Box::new(InsightsData {
        used: size_words(used),
        total: size_words(quota),
        plan: format!("{} plan", size_words(quota)),
        share,
        used_says: format!("{}% used", (share * 100.0).round() as u32),
        free_says: format!("{} free", size_words((quota - used).max(0))),
        totals: format!(
            "{} · {}",
            count_words(files.len(), "file", "files"),
            size_words(total_bytes)
        ),
        kinds: kinds
            .iter()
            .map(|(name, bytes, n)| KindShare {
                name: (*name).to_string(),
                says: format!("{} · {}", size_words(*bytes), count_words(*n, "file", "files")),
                share: *bytes as f32 / largest_kind,
            })
            .collect(),
        largest_says: format!("top {}", largest.len()),
        largest: largest
            .iter()
            .take(LARGEST_SHOWN)
            .map(|f| {
                let path = f.path.trim_matches('/').to_string();
                LargeFile {
                    shown: cut_middle(&path),
                    kind: kind_name(&path).to_string(),
                    size: size_words(f.size_bytes),
                    path,
                }
            })
            .collect(),
        more,
    }))
}

/// A path cut in the middle so the file name stays whole ("Backups/Anna's…ook Pro/Documents/Archive 2024.zip").
pub(crate) fn cut_middle(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= PATH_SHOWN {
        return path.to_string();
    }
    let name = path.rsplit('/').next().unwrap_or(path).chars().count();
    let head = 14.min(PATH_SHOWN.saturating_sub(name + 1));
    let tail = (PATH_SHOWN - head - 1).max(name);
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail.min(chars.len())..].iter().collect();
    format!("{start}\u{2026}{end}")
}

/// "240 GB", "3.1 GB", "940 MB": decimal units, one decimal where it says something.
pub(crate) fn size_words(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    let (value, unit) = [(1e12, "TB"), (1e9, "GB"), (1e6, "MB"), (1e3, "KB")]
        .into_iter()
        .find(|(size, _)| bytes >= *size)
        .map_or((bytes, "bytes"), |(size, unit)| (bytes / size, unit));
    if unit == "bytes" {
        return format!("{} bytes", value as u64);
    }
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 || rounded >= 1000.0 {
        format!("{} {unit}", rounded.round() as u64)
    } else {
        format!("{rounded:.1} {unit}")
    }
}

/// "1 file", "3,140 files".
pub(crate) fn count_words(n: usize, one: &str, many: &str) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{grouped} {}", if n == 1 { one } else { many })
}

/// Keep on this PC's folder tree from the live rows: by path (a live row's place is its path), each folder with
/// what it holds, its size and whether it is kept.
pub(crate) fn keep_of(rows: &[(FileEntry, bool, DriveItem)]) -> Keep {
    let trim = |p: &str| p.trim_matches('/').to_string();
    let parent = |p: &str| p.rsplit_once('/').map(|(head, _)| head.to_string()).unwrap_or_default();
    let mut bytes: HashMap<String, i64> = HashMap::new();
    let mut holds: HashMap<String, usize> = HashMap::new();
    let mut total: i64 = 0;
    for (entry, _, _) in rows {
        let path = trim(&entry.path);
        *holds.entry(parent(&path)).or_default() += 1;
        if entry.is_dir() {
            continue;
        }
        let size = entry.size_bytes.max(0);
        total += size;
        let mut up = parent(&path);
        while !up.is_empty() {
            *bytes.entry(up.clone()).or_default() += size;
            up = parent(&up);
        }
    }
    let folders: Vec<(String, &FileEntry, bool, &DriveItem)> = rows
        .iter()
        .filter(|(e, _, _)| e.is_dir())
        .map(|(e, kept, item)| (trim(&e.path), e, *kept, item))
        .collect();
    fn level(
        at: &str,
        folders: &[(String, &FileEntry, bool, &DriveItem)],
        bytes: &HashMap<String, i64>,
        holds: &HashMap<String, usize>,
        parent: &dyn Fn(&str) -> String,
    ) -> Vec<Folder> {
        let mut out: Vec<Folder> = folders
            .iter()
            .filter(|(path, _, _, _)| parent(path) == at)
            .map(|(path, entry, kept, item)| {
                let children = level(path, folders, bytes, holds, parent);
                Folder {
                    id: entry.file_id.clone(),
                    name: item.name.clone(),
                    count: count_words(holds.get(path).copied().unwrap_or(0), "item", "items"),
                    mb: bytes.get(path).copied().unwrap_or(0) as f32 / 1e6,
                    kept: *kept,
                    expandable: !children.is_empty(),
                    open: false,
                    children,
                }
            })
            .collect();
        out.sort_by_key(|f| f.name.to_lowercase());
        out
    }
    Keep::Ready {
        folders: level("", &folders, &bytes, &holds, &parent),
        total_mb: total as f32 / 1e6,
    }
}
