//! What the studio is doing to the engine it manages, in the two sentences the person reads (Sprint 28h, S2a, D35).
//!
//! "downloading engine X" while an archive is coming down, and "engine X installed, takes effect when the services
//! restart" once it is in place. Both carry the time they began, both are written to the studio log, and both are shown
//! in the Settings summary line. The state is held in memory on purpose: it describes this run of the studio, and a line
//! that survived a restart would claim a download that is no longer running.

use std::sync::{Arc, Mutex};

use crate::studio_log::utc_stamp;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Activity {
    Downloading { version: String, at_millis: u64 },
    Installed { version: String, at_millis: u64 },
}

static CURRENT: Mutex<Option<Activity>> = Mutex::new(None);
static NOTIFIER: Mutex<Option<Arc<dyn Fn() + Send + Sync>>> = Mutex::new(None);

/// Registers what is called after every change, so a window can reload without waiting to be asked.
pub fn set_notifier(notifier: Arc<dyn Fn() + Send + Sync>) {
    if let Ok(mut slot) = NOTIFIER.lock() {
        *slot = Some(notifier);
    }
}

/// An archive for `version` has started coming down (or resumed).
pub fn downloading(version: &str) {
    record(Activity::Downloading { version: version.to_string(), at_millis: crate::field_view::now_millis() });
}

/// `version` is installed and will be used when the services next start.
pub fn installed(version: &str) {
    record(Activity::Installed { version: version.to_string(), at_millis: crate::field_view::now_millis() });
}

/// A service was started, so an engine installed before now is the one that runs: its notice no longer applies.
pub fn services_started() {
    let cleared = match CURRENT.lock() {
        Ok(mut current) => {
            if matches!(*current, Some(Activity::Installed { .. })) {
                *current = None;
                true
            } else {
                false
            }
        }
        Err(_) => false,
    };
    if cleared {
        notify();
    }
}

/// The sentence to show, or none when the studio is doing nothing to the engine.
pub fn line() -> Option<String> {
    CURRENT.lock().ok().and_then(|current| current.as_ref().map(sentence))
}

/// The failed or interrupted download is not "downloading" any more.
pub fn download_ended() {
    let cleared = match CURRENT.lock() {
        Ok(mut current) => {
            if matches!(*current, Some(Activity::Downloading { .. })) {
                *current = None;
                true
            } else {
                false
            }
        }
        Err(_) => false,
    };
    if cleared {
        notify();
    }
}

fn sentence(activity: &Activity) -> String {
    match activity {
        Activity::Downloading { version, at_millis } => format!("{} downloading engine {version}", utc_stamp(*at_millis)),
        Activity::Installed { version, at_millis } => {
            format!("{} engine {version} installed, takes effect when the services restart", utc_stamp(*at_millis))
        }
    }
}

fn record(activity: Activity) {
    crate::studio_log::say(&sentence(&activity));
    if let Ok(mut current) = CURRENT.lock() {
        *current = Some(activity);
    }
    notify();
}

fn notify() {
    let notifier = NOTIFIER.lock().ok().and_then(|slot| slot.clone());
    if let Some(notifier) = notifier {
        notifier();
    }
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    if let Ok(mut current) = CURRENT.lock() {
        *current = None;
    }
}

#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_sentences_name_the_engine_and_carry_a_time() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        assert_eq!(line(), None, "nothing is being done to the engine");

        downloading("4.4.20");
        let during = line().expect("a download is under way");
        assert!(during.contains("downloading engine 4.4.20"), "{during}");
        assert!(during.starts_with("20"), "no time in front: {during}");

        installed("4.4.20");
        let after = line().expect("the engine is installed");
        assert!(after.contains("engine 4.4.20 installed, takes effect when the services restart"), "{after}");
        assert!(after.starts_with("20"), "no time in front: {after}");
        reset_for_test();
    }

    #[test]
    fn starting_a_service_retires_the_installed_notice_and_a_failed_download_retires_its_own() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        installed("4.4.20");
        services_started();
        assert_eq!(line(), None, "the notice outlived the restart it promised");

        downloading("4.4.21");
        services_started();
        assert!(line().is_some(), "CONTROL: starting a service must not hide a download that is under way");
        download_ended();
        assert_eq!(line(), None, "an ended download still reads as running");
        reset_for_test();
    }

    #[test]
    fn every_change_calls_the_notifier() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        set_notifier(Arc::new(move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        downloading("4.4.20");
        installed("4.4.20");
        services_started();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
        reset_for_test();
    }
}
