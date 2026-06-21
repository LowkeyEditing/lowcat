use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

pub fn command(tool: &str) -> Command {
    Command::new(resolve(tool).unwrap_or_else(|| PathBuf::from(tool)))
}

pub fn available(tool: &str) -> bool {
    command(tool)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn resolve(tool: &str) -> Option<PathBuf> {
    search_path(tool).or_else(|| search_common_dirs(tool))
}

fn search_path(tool: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|dir| dir.join(tool))
        .find(|candidate| is_executable(candidate))
}

fn search_common_dirs(tool: &str) -> Option<PathBuf> {
    common_tool_dirs()
        .iter()
        .map(|dir| dir.join(tool))
        .find(|candidate| is_executable(candidate))
}

#[cfg(target_os = "macos")]
fn common_tool_dirs() -> &'static [PathBuf] {
    use std::sync::OnceLock;

    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| {
        vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
        ]
    })
}

#[cfg(not(target_os = "macos"))]
fn common_tool_dirs() -> &'static [PathBuf] {
    &[]
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        path.metadata()
            .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        path.is_file()
    }
}
