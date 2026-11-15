//! First-run step 4 (DW-0204, and its compare view DW-0209) on the real engine: photo previews.
//!
//! - The step reads the account's preview setting, the stored previews and their averages once
//!   ([`SetupEngine::previews_facts_for`], on a worker). While that is out the step is a loading state; a read that
//!   failed says so with Try again (never "off", never "not in this version"); once answered the step starts on the
//!   account's STORED size and format ([`SetupSource::stored_previews`]), so Continue on untouched choices changes
//!   nothing but turning previews on (decision 2527; an account whose previews are already on sees its own choice).
//! - The sample: the newest of the person's own photos on this PC that decodes here, measured on this PC
//!   ([`SetupEngine::measure_sample_for`]); "Choose photo" opens Windows' file picker in the sync folder and measures
//!   the picked file instead. With none, the picture that ships inside the app, said as that. Nothing is uploaded.
//! - A weight is what a JPEG preview of the sample weighs, encoded on this PC; with HEIC (no encoder here), the
//!   account's own average where it has one. The line under the choices is the account's previews at that choice,
//!   only from figures that exist; no figure, no line. No "Space on this PC": nothing is kept here yet (PREV-2).
//! - Continue writes `{enabled: true, format, px}` for the session of the press; the run that makes previews starts
//!   once it was stored (the app's one run, `drive_previews::shared_run`). "Not now" writes nothing.
//!
//! Must not: log a path or an id; hold the person's pictures for another session (the shell drops this source with
//! its session).

use beebeeb_engine::app::photo_previews_api::{PreviewsFacts, PreviewsRefused};
use beebeeb_engine::photo_previews::{PreviewFormat as Wire, PreviewSetting};

use super::*;
use crate::engine_host::drive_previews::{
    Measured, bytes_words, format_of, grouped, px_of, shared_run, size_of, wire_of,
};
use crate::frames::settings::previews::NOT_MEASURED;

/// What step 4 holds.
#[derive(Default)]
pub(super) struct StepFour {
    /// The engine binds previews and the read was asked.
    pub(super) asked: bool,
    pub(super) reading: bool,
    read: Option<Result<PreviewsFacts, PreviewsRefused>>,
    sample: Option<Measured>,
    pub(super) measuring: bool,
    /// This PC's space limit in bytes, read once when step 4 first asks for it (`None`: none known).
    limit: std::cell::OnceCell<Option<u64>>,
}

impl StepFour {
    fn facts(&self) -> Option<&PreviewsFacts> {
        self.read.as_ref().and_then(|read| read.as_ref().ok())
    }

    pub(super) fn failed(&self) -> bool {
        !self.reading && matches!(self.read, Some(Err(_)))
    }
}

impl<E: SetupEngine> EngineSetupSource<E> {
    /// Read step 4's facts on a worker (once per press of Try again; never twice at once).
    pub(super) fn read_previews(&mut self) {
        if !self.engine.binds_previews() || !self.mine() || self.previews.reading {
            return;
        }
        self.previews.asked = true;
        self.previews.reading = true;
        self.previews.read = None;
        self.ask(
            "first-run-previews-read",
            |engine, session| Answer::PreviewsRead(engine.previews_facts_for(session)),
            Some(Answer::PreviewsRead(Err(PreviewsRefused::Failed))),
        );
    }

    /// The read answered; once it worked, the sample is measured (once).
    pub(super) fn took_previews_read(&mut self, read: Result<PreviewsFacts, PreviewsRefused>) {
        self.previews.reading = false;
        if let Err(why) = &read {
            log(format!("first run: the preview setting was not read ({})", why.kind()));
        }
        let worked = read.is_ok();
        self.previews.read = Some(read);
        if worked && self.previews.sample.is_none() && !self.previews.measuring {
            self.previews.measuring = true;
            self.ask(
                "first-run-previews-sample",
                |engine, session| {
                    Answer::PreviewsSample(
                        engine
                            .measure_sample_for(session, None)
                            .map(Measured::of)
                            .map_err(|_| ()),
                    )
                },
                Some(Answer::PreviewsSample(Err(()))),
            );
        }
    }

    /// A sample was measured (`Ok(None)`: no photo here decodes; `Err`: nothing changes).
    pub(super) fn took_previews_sample(&mut self, sample: Result<Option<Measured>, ()>) {
        self.previews.measuring = false;
        if let Ok(sample) = sample {
            self.previews.sample = sample;
        }
    }

    fn stored_setting(&self) -> Option<PreviewSetting> {
        self.previews.facts().map(|facts| facts.setting)
    }

    pub(super) fn previews_stored(&self) -> Option<(PreviewSize, PreviewFormat)> {
        let setting = self.stored_setting()?;
        Some((
            size_of(setting.px).unwrap_or(PreviewSize::K2),
            format_of(setting.format),
        ))
    }

    /// Step 4's "Space on this PC": the engine's limit (read once), in the board's words; empty when none is known.
    pub(super) fn previews_space_limit(&self) -> String {
        let bytes = *self.previews.limit.get_or_init(|| self.engine.preview_limit_bytes());
        bytes.map_or_else(String::new, bytes_words)
    }

    /// The account's own average at a format and long edge, where it has previews there.
    fn average(&self, format: Wire, px: u32) -> Option<u64> {
        self.previews
            .facts()?
            .averages
            .iter()
            .find(|(at, _, _)| *at == (format, px))
            .map(|(_, bytes, _)| *bytes)
    }

    /// JPEG: measured here on the sample; HEIC: the account's own average.
    fn bytes_at(&self, size: PreviewSize, format: PreviewFormat) -> Option<u64> {
        let px = px_of(size);
        match wire_of(format) {
            Wire::Jpeg => self.previews.sample.as_ref()?.weight(px),
            Wire::Heic => self.average(Wire::Heic, px),
        }
    }

    pub(super) fn previews_original_weight(&self) -> String {
        if !self.previews.asked {
            return String::new();
        }
        self.previews
            .sample
            .as_ref()
            .map_or_else(|| NOT_MEASURED.to_string(), |sample| bytes_words(sample.original_bytes))
    }

    pub(super) fn previews_weight(&self, size: PreviewSize, format: PreviewFormat) -> Option<String> {
        self.bytes_at(size, format).map(bytes_words)
    }

    /// "About 1.2 GB in your Beebeeb storage for 2,431 photos.": the account's previews at that choice. No figure
    /// (no previews stored, nothing measured at that choice): no line.
    pub(super) fn previews_fit_line(&self, size: PreviewSize, format: PreviewFormat) -> String {
        let Some(count) = self
            .previews
            .facts()
            .map(|facts| facts.summary.count)
            .filter(|n| *n > 0)
        else {
            return String::new();
        };
        let px = px_of(size);
        let Some(each) = self
            .average(wire_of(format), px)
            .or_else(|| self.bytes_at(size, format))
        else {
            return String::new();
        };
        format!(
            "About {} in your Beebeeb storage for {} photos.",
            bytes_words(each.saturating_mul(count)),
            grouped(count)
        )
    }

    pub(super) fn previews_sample(&self) -> Sample {
        match (&self.previews.sample, self.stored_setting()) {
            (Some(sample), Some(setting)) => sample.sample(setting.px, setting.format),
            _ => Sample::bundled(),
        }
    }

    /// Continue on step 4 / Done in its compare view: store `{enabled: true, format, px}` for the session of the
    /// press, then start the run. Answered as `Switch::Previews`.
    pub(super) fn store_previews(&mut self, size: PreviewSize, format: PreviewFormat) {
        let setting = PreviewSetting {
            enabled: true,
            format: wire_of(format),
            px: px_of(size),
        };
        let not_done = Outcome {
            switch: Switch::Previews,
            done: false,
            now: None,
        };
        let engine = self.engine.clone();
        self.ask(
            "first-run-previews",
            move |handle, session| match handle.set_previews_for(session, setting) {
                Ok(()) => {
                    let run = shared_run();
                    let waker = engine.clone();
                    run.on_change(Arc::new(move || waker.wake()));
                    let started = run.start(
                        session,
                        Box::new(move |stop, progress| engine.make_previews(session, false, stop, progress)),
                    );
                    if !started {
                        log("first run: the preview run could not start");
                    }
                    Answer::Switched(Outcome {
                        done: true,
                        now: Some(true),
                        ..not_done
                    })
                }
                Err(why) => {
                    log(format!("first run: previews were not turned on ({})", why.kind()));
                    Answer::Switched(not_done)
                }
            },
            Some(Answer::Switched(not_done)),
        );
    }

    /// "Choose photo": Windows' file picker in the sync folder; the picked photo is measured on this PC. One at a time.
    pub(super) fn pick_previews_photo(&mut self) {
        if self.previews.measuring || !self.mine() || !self.previews.asked {
            return;
        }
        self.previews.measuring = true;
        let platform = self.hooks.platform.clone();
        self.ask(
            "first-run-photo",
            move |engine, session| {
                let start = engine.sync_folder().unwrap_or_default();
                match platform.pick_photo("Choose a photo to compare", &start) {
                    Ok(Some(picked)) => {
                        Answer::PreviewsSample(match engine.measure_sample_for(session, Some(picked)) {
                            Ok(facts) => Ok(Measured::of(facts)).and_then(|measured| measured.map(Some).ok_or(())),
                            Err(_) => {
                                log("first run: the chosen photo could not be measured");
                                Err(())
                            }
                        })
                    }
                    Ok(None) => Answer::PreviewsSample(Err(())),
                    Err(_) => {
                        log("first run: the photo picker did not open");
                        Answer::PreviewsSample(Err(()))
                    }
                }
            },
            Some(Answer::PreviewsSample(Err(()))),
        );
    }
}
