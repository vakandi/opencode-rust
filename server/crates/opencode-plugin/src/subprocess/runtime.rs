//! JS runtime detection — finds bun, deno, or node on `$PATH`.

use std::path::PathBuf;

/// Supported JavaScript runtimes, in order of preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsRuntime {
    Bun,
    Deno,
    Node,
}

impl JsRuntime {
    /// The executable name.
    pub fn command(&self) -> &'static str {
        match self {
            Self::Bun => "bun",
            Self::Deno => "deno",
            Self::Node => "node",
        }
    }

    /// Arguments to run a TS file.
    pub fn run_args(&self, script: &str) -> Vec<String> {
        match self {
            Self::Bun => vec!["run".into(), script.into()],
            Self::Deno => vec!["run".into(), "--allow-all".into(), script.into()],
            Self::Node => {
                // Node >=22 has native TS support via --experimental-strip-types
                vec!["--experimental-strip-types".into(), script.into()]
            }
        }
    }

    /// The package manager command to use for `npm install`.
    /// For Bun/Deno we use their built-in install; for Node we use npm.
    pub fn install_command(&self) -> &'static str {
        match self {
            Self::Bun => "bun",
            Self::Deno => "deno",
            Self::Node => "npm",
        }
    }

    /// Arguments for the install command.
    pub fn install_args(&self) -> Vec<String> {
        vec!["install".into()]
    }
}

/// Detect the best available JS runtime, preferring bun > deno > node.
pub fn detect_runtime() -> Option<JsRuntime> {
    for rt in [JsRuntime::Bun, JsRuntime::Deno, JsRuntime::Node] {
        if which::which(rt.command()).is_ok() {
            return Some(rt);
        }
    }
    None
}

/// Return the full path to the runtime binary, if found.
pub fn runtime_path(rt: JsRuntime) -> Option<PathBuf> {
    which::which(rt.command()).ok()
}
