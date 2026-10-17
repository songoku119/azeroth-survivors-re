//! The Android-only half of the shim: the app's directories, the environment, the panic hook.

use std::ffi::CString;
use std::fs;
use std::path::PathBuf;

use android_log_sys::LogPriority;

use crate::env_shim;

/// The logcat tag, for `adb logcat -s wsurvivors:*`.
const TAG: &str = "wsurvivors";

/// Everything that has to be true before `run_survivors`: the panic hook (first, so a failure
/// below still reaches logcat), then the environment from the app's directories and `env.toml`.
pub fn prepare() {
    install_panic_hook();

    let app = bevy::android::ANDROID_APP
        .get()
        .expect("android_main sets ANDROID_APP before main");
    // `internal_data_path` is `getFilesDir()`; it is `None` only when the activity is gone.
    let internal = app
        .internal_data_path()
        .unwrap_or_else(|| PathBuf::from("/data/local/tmp"));
    let external = app.external_data_path();

    let home = internal.join("benilla-config");
    if let Err(e) = fs::create_dir_all(&home) {
        log(LogPriority::WARN, &format!("cannot create {}: {e}", home.display()));
    }
    let env_toml = fs::read_to_string(home.join("env.toml")).unwrap_or_default();
    let data = env_shim::data_dir(&internal, external.as_deref(), |p| p.is_dir());
    let vars = env_shim::variables(&home, &data, &env_toml, |k| std::env::var_os(k).is_some());
    for (key, value) in vars {
        log(LogPriority::INFO, &format!("env {key}={value}"));
        std::env::set_var(key, value);
    }
}

/// Chain a hook that writes the panic to logcat, then runs the previous one (the default
/// stderr print; `crash::install` chains its file writer on top of this one later).
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log(LogPriority::FATAL, &format!("panic: {info}"));
        previous(info);
    }));
}

/// One line to logcat under [`TAG`]. An interior NUL would make `CString` fail; it is replaced
/// rather than dropping the line.
fn log(priority: LogPriority, text: &str) {
    let tag = CString::new(TAG).expect("static tag has no NUL");
    let text = CString::new(text.replace('\0', "?")).expect("NULs replaced");
    // SAFETY: both pointers are live NUL-terminated C strings for the duration of the call.
    unsafe {
        android_log_sys::__android_log_write(priority as i32, tag.as_ptr(), text.as_ptr());
    }
}
