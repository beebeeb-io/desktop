//! The repair run and the open's thumbnails against a fake engine: no helper, no network. Every id is invented.

use std::sync::atomic::AtomicU32;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;
use crate::engine_host::thumbs::Thumbs;

const IDS: [&str; 3] = [
    "e0000000-0000-4000-8000-000000000001",
    "e0000000-0000-4000-8000-000000000002",
    "e0000000-0000-4000-8000-000000000003",
];

#[derive(Default)]
struct Fake {
    /// Each file: its id, and what the local state says of its server thumbnail.
    files: Mutex<Vec<(String, Option<bool>)>>,
    hold: Mutex<Option<MadeUpload>>,
    downloads: AtomicU32,
    /// Medium uploads, by file id, with the 404 the send was given.
    sent: Mutex<Vec<(String, bool)>>,
    /// While set, a download waits for a word on it (taken by the first download only).
    gate: Mutex<Option<mpsc::Receiver<()>>>,
    began: Mutex<Option<mpsc::Sender<()>>>,
}

impl Fake {
    fn with(files: &[(&str, Option<bool>)]) -> Arc<Self> {
        let fake = Fake::default();
        *fake.files.lock().unwrap() = files.iter().map(|(id, has)| (id.to_string(), *has)).collect();
        Arc::new(fake)
    }

    fn sent(&self) -> Vec<(String, bool)> {
        self.sent.lock().unwrap().clone()
    }

    fn flag(&self, id: &str) -> Option<bool> {
        self.files
            .lock()
            .unwrap()
            .iter()
            .find(|(f, _)| f == id)
            .and_then(|(_, has)| *has)
    }
}

impl ThumbMaker for Fake {
    fn known(&self, _: SessionGeneration, file_id: &str) -> Option<bool> {
        self.flag(file_id)
    }

    fn server_has_none(&self, _: SessionGeneration, _: &str) -> bool {
        true
    }

    fn send(&self, _: SessionGeneration, file_id: &str, thumbs: &Thumbs, server_404: bool) -> MadeUpload {
        assert!(thumbs.width > 0 && thumbs.rgba.len() == (thumbs.width * thumbs.height * 4) as usize);
        if self.flag(file_id) == Some(true) {
            return MadeUpload::NothingToSend;
        }
        self.sent.lock().unwrap().push((file_id.to_string(), server_404));
        // The engine marks has_thumbnail once the medium thumbnail is on the server.
        for (id, has) in self.files.lock().unwrap().iter_mut() {
            if id == file_id {
                *has = Some(true);
            }
        }
        MadeUpload::Sent
    }
}

impl RepairEngine for Fake {
    fn session(&self) -> SessionGeneration {
        SessionGeneration::for_test(1)
    }

    fn wake(&self) {}

    fn hold(&self, _: SessionGeneration) -> Option<MadeUpload> {
        *self.hold.lock().unwrap()
    }

    fn missing(&self, _: SessionGeneration) -> Option<Vec<MissingThumb>> {
        Some(
            self.files
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, has)| *has != Some(true))
                .map(|(id, has)| MissingThumb {
                    file_id: id.clone(),
                    name: "Invented photo.jpg".into(),
                    server_said_none: *has == Some(false),
                })
                .collect(),
        )
    }

    fn download(&self, _: SessionGeneration, _: &str, _: &AtomicBool) -> Option<zeroize::Zeroizing<Vec<u8>>> {
        self.downloads.fetch_add(1, Ordering::SeqCst);
        if let Some(began) = self.began.lock().unwrap().as_ref() {
            let _ = began.send(());
        }
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.recv_timeout(Duration::from_secs(10));
        }
        Some(zeroize::Zeroizing::new(b"invented picture bytes".to_vec()))
    }
}

fn picture(_: &[u8], mime: &str) -> Option<Thumbs> {
    assert!(mime.starts_with("image/"));
    Some(Thumbs {
        width: 4,
        height: 3,
        rgba: zeroize::Zeroizing::new(vec![200; 4 * 3 * 4]),
    })
}

fn ended(repair: &ThumbRepair) -> Status {
    let began = Instant::now();
    while repair.status().running() {
        assert!(began.elapsed() < Duration::from_secs(10), "the run ends");
        std::thread::sleep(Duration::from_millis(5));
    }
    repair.join();
    repair.status()
}

#[test]
fn thumb_opened_file_with_none_ends_with_a_medium_upload_and_has_thumbnail_true() {
    let fake = Fake::with(&[(IDS[0], Some(false))]);
    let made = make_from_bytes(
        &*fake,
        SessionGeneration::for_test(1),
        IDS[0],
        "image/jpeg",
        b"bytes",
        picture,
    );
    assert_eq!(made, Some(MadeUpload::Sent));
    assert_eq!(fake.sent(), vec![(IDS[0].to_string(), false)]);
    assert_eq!(fake.flag(IDS[0]), Some(true));
    // Opened again: the server has one now, nothing is made.
    let again = make_from_bytes(
        &*fake,
        SessionGeneration::for_test(1),
        IDS[0],
        "image/jpeg",
        b"bytes",
        picture,
    );
    assert_eq!(again, None);
    assert_eq!(fake.sent().len(), 1);
}

#[test]
fn thumb_open_of_a_file_not_known_asks_the_server_and_sends_with_its_404() {
    let fake = Fake::with(&[(IDS[0], None)]);
    let made = make_from_bytes(
        &*fake,
        SessionGeneration::for_test(1),
        IDS[0],
        "image/png",
        b"bytes",
        picture,
    );
    assert_eq!(made, Some(MadeUpload::Sent));
    assert_eq!(fake.sent(), vec![(IDS[0].to_string(), true)]);
}

#[test]
fn thumb_open_of_a_document_makes_nothing() {
    let fake = Fake::with(&[(IDS[0], Some(false))]);
    let made = make_from_bytes(
        &*fake,
        SessionGeneration::for_test(1),
        IDS[0],
        "application/pdf",
        b"%PDF",
        picture,
    );
    assert_eq!(made, None);
    assert!(fake.sent().is_empty());
}

#[test]
fn thumb_repair_over_three_files_uploads_three() {
    let fake = Fake::with(&[(IDS[0], Some(false)), (IDS[1], None), (IDS[2], Some(false))]);
    let repair = ThumbRepair::with_generate(fake.clone(), picture);
    assert!(repair.start());
    let status = ended(&repair);
    assert_eq!(status.phase, Phase::Done);
    assert_eq!((status.done, status.total, status.made), (3, 3, 3));
    let mut sent: Vec<String> = fake.sent().into_iter().map(|(id, _)| id).collect();
    sent.sort();
    assert_eq!(sent, IDS.map(String::from).to_vec());
    assert!(IDS.iter().all(|id| fake.flag(id) == Some(true)));
}

#[test]
fn thumb_repair_never_repeats_a_file_in_one_run() {
    let fake = Fake::with(&[(IDS[0], Some(false)), (IDS[0], Some(false)), (IDS[1], Some(false))]);
    let repair = ThumbRepair::with_generate(fake.clone(), picture);
    assert!(repair.start());
    let status = ended(&repair);
    assert_eq!((status.total, status.made), (2, 2));
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 2);
    assert_eq!(fake.sent().len(), 2);
}

#[test]
fn thumb_repair_stops_cleanly_on_stop() {
    let fake = Fake::with(&[(IDS[0], Some(false)), (IDS[1], Some(false)), (IDS[2], Some(false))]);
    let (open, gate) = mpsc::channel();
    let (began, has_begun) = mpsc::channel();
    *fake.gate.lock().unwrap() = Some(gate);
    *fake.began.lock().unwrap() = Some(began);
    let repair = ThumbRepair::with_generate(fake.clone(), picture);
    assert!(repair.start());
    // The first download is in hand (and held), the second worker may have begun one too: Stop.
    has_begun
        .recv_timeout(Duration::from_secs(10))
        .expect("a download began");
    repair.stop();
    open.send(()).unwrap();
    let status = ended(&repair);
    assert_eq!(status.phase, Phase::Stopped);
    // At most the files in hand when Stop was pressed (one per worker) were sent; never all three.
    let downloads = fake.downloads.load(Ordering::SeqCst);
    assert!(downloads < 3, "no file is begun after Stop");
    assert_eq!(fake.sent().len() as u32, downloads);
    assert_eq!(status.made, downloads);
    // A new run after a stop goes on with what is left.
    assert!(repair.start());
    let status = ended(&repair);
    assert_eq!(status.phase, Phase::Done);
    assert_eq!(fake.sent().len(), 3);
}

#[test]
fn thumb_repair_is_blocked_by_metered() {
    let fake = Fake::with(&[(IDS[0], Some(false)), (IDS[1], Some(false))]);
    *fake.hold.lock().unwrap() = Some(MadeUpload::NotNow(NotNow::Hold(PausedFor::Metered)));
    let repair = ThumbRepair::with_generate(fake.clone(), picture);
    assert!(repair.start());
    let status = ended(&repair);
    assert_eq!(
        status.phase,
        Phase::Held(held_words(MadeUpload::NotNow(NotNow::Hold(PausedFor::Metered))))
    );
    assert_eq!(
        fake.downloads.load(Ordering::SeqCst),
        0,
        "nothing is downloaded on a metered network"
    );
    assert!(fake.sent().is_empty());
}

#[test]
fn thumb_repair_respects_the_setting_and_the_pause() {
    for why in [MadeUpload::NotNow(NotNow::SettingOff), MadeUpload::Paused] {
        let fake = Fake::with(&[(IDS[0], Some(false))]);
        *fake.hold.lock().unwrap() = Some(why);
        let repair = ThumbRepair::with_generate(fake.clone(), picture);
        assert!(repair.start());
        assert_eq!(ended(&repair).phase, Phase::Held(held_words(why)));
        assert_eq!(fake.downloads.load(Ordering::SeqCst), 0);
    }
}
