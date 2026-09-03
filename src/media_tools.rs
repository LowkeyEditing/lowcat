use std::{
    env, io,
    path::{Path, PathBuf},
    process::Command,
};

const REQUIRED_TOOLS: &[&str] = &["ffmpeg", "ffprobe", "yt-dlp"];

#[derive(Debug, Clone)]
pub struct MissingTool {
    pub name: &'static str,
    pub search_locations: Vec<SearchLocation>,
}

#[derive(Debug, Clone)]
pub enum SearchLocation {
    Path,
    Directory(PathBuf),
}

pub fn command(tool: &str) -> Command {
    let executable = resolve(tool).unwrap_or_else(|| PathBuf::from(tool));

    #[cfg(target_os = "windows")]
    if executable
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cmd"))
        || executable
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("bat"))
    {
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/C"]).arg(executable);
        suppress_windows_console(&mut command);
        return command;
    }

    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new(executable);
        suppress_windows_console(&mut command);
        return command;
    }

    Command::new(executable)
}

#[cfg(target_os = "windows")]
fn suppress_windows_console(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

pub(crate) fn probe_duration_seconds(path: &Path) -> Option<f64> {
    let output = command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let duration = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()?;
    (duration.is_finite() && duration > 0.).then_some(duration)
}

pub(crate) fn probe_audio_channels(path: &Path) -> io::Result<usize> {
    let output = command("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=channels",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("ffprobe channel probe failed"));
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<usize>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(crate) fn has_audio_stream(path: &Path) -> bool {
    command("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .map(|output| output.status.success() && !output.stdout.is_empty())
        .unwrap_or(false)
}

pub fn available(tool: &str) -> bool {
    command(tool)
        .arg(version_arg(tool))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn missing_required_tools() -> Vec<MissingTool> {
    REQUIRED_TOOLS
        .iter()
        .copied()
        .filter(|tool| !available(tool))
        .map(|tool| MissingTool {
            name: tool,
            search_locations: display_search_locations(),
        })
        .collect()
}

pub fn resolve(tool: &str) -> Option<PathBuf> {
    search_path(tool).or_else(|| search_common_dirs(tool))
}

fn display_search_locations() -> Vec<SearchLocation> {
    let mut locations = Vec::new();
    if env::var_os("PATH").is_some() {
        locations.push(SearchLocation::Path);
    }
    for dir in common_tool_dirs() {
        push_unique_location(&mut locations, dir.clone());
    }
    locations
}

fn search_path(tool: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths).find_map(|dir| find_in_dir(&dir, tool))
}

fn search_common_dirs(tool: &str) -> Option<PathBuf> {
    common_tool_dirs()
        .iter()
        .find_map(|dir| find_in_dir(dir, tool))
}

fn find_in_dir(dir: &Path, tool: &str) -> Option<PathBuf> {
    let direct = dir.join(tool);
    if is_executable(&direct) {
        return Some(direct);
    }

    #[cfg(target_os = "windows")]
    if Path::new(tool).extension().is_none() {
        for extension in windows_executable_extensions() {
            let mut name = std::ffi::OsString::from(tool);
            name.push(extension);
            let candidate = dir.join(name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn windows_executable_extensions() -> Vec<String> {
    let mut extensions = vec![".exe".to_string()];
    let Some(path_ext) = env::var_os("PATHEXT") else {
        return extensions;
    };

    for extension in path_ext.to_string_lossy().split(';') {
        let extension = extension.trim();
        if extension.is_empty() {
            continue;
        }
        let extension = if extension.starts_with('.') {
            extension.to_string()
        } else {
            format!(".{extension}")
        };
        if !extensions
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&extension))
        {
            extensions.push(extension);
        }
    }
    extensions
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

#[cfg(target_os = "windows")]
fn common_tool_dirs() -> &'static [PathBuf] {
    use std::sync::OnceLock;

    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| {
        let mut dirs = Vec::new();

        if let Ok(executable) = env::current_exe()
            && let Some(app_dir) = executable.parent()
        {
            push_unique_path(&mut dirs, app_dir.to_path_buf());
            push_unique_path(&mut dirs, app_dir.join("bin"));
            push_unique_path(&mut dirs, app_dir.join("tools"));
        }

        if let Some(ffmpeg_home) = env::var_os("FFMPEG_HOME") {
            push_unique_path(&mut dirs, PathBuf::from(ffmpeg_home).join("bin"));
        }

        if let Some(chocolatey) = env::var_os("ChocolateyInstall") {
            push_unique_path(&mut dirs, PathBuf::from(chocolatey).join("bin"));
        } else if let Some(program_data) = env::var_os("ProgramData") {
            push_unique_path(
                &mut dirs,
                PathBuf::from(program_data).join("chocolatey").join("bin"),
            );
        }

        if let Some(scoop) = env::var_os("SCOOP") {
            push_unique_path(&mut dirs, PathBuf::from(scoop).join("shims"));
        } else if let Some(user_profile) = env::var_os("USERPROFILE") {
            push_unique_path(
                &mut dirs,
                PathBuf::from(user_profile).join("scoop").join("shims"),
            );
        }

        if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
            let local_app_data = PathBuf::from(local_app_data);
            push_unique_path(
                &mut dirs,
                local_app_data
                    .join("Microsoft")
                    .join("WinGet")
                    .join("Links"),
            );
            push_unique_path(
                &mut dirs,
                local_app_data.join("Microsoft").join("WindowsApps"),
            );
            push_python_script_dirs(&mut dirs, &local_app_data.join("Programs").join("Python"));
        }

        if let Some(app_data) = env::var_os("APPDATA") {
            push_python_script_dirs(&mut dirs, &PathBuf::from(app_data).join("Python"));
        }

        // A GUI app can be launched by a long-running shell/editor whose
        // environment predates a PATH change. Include the persisted Windows
        // PATH values so installed tools remain discoverable in that case.
        for path in persisted_windows_path_dirs() {
            push_unique_path(&mut dirs, path);
        }

        dirs
    })
}

#[cfg(target_os = "windows")]
fn persisted_windows_path_dirs() -> Vec<PathBuf> {
    use winreg::{RegKey, enums::*};

    let mut dirs = Vec::new();
    let keys = [
        (HKEY_CURRENT_USER, "Environment"),
        (
            HKEY_LOCAL_MACHINE,
            "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
        ),
    ];
    for (hkey, subkey) in keys {
        let Ok(key) = RegKey::predef(hkey).open_subkey(subkey) else {
            continue;
        };
        let Ok(path) = key.get_value::<String, _>("Path") else {
            continue;
        };
        for dir in env::split_paths(std::ffi::OsStr::new(&path)) {
            push_unique_path(&mut dirs, dir);
        }
    }
    dirs
}

#[cfg(target_os = "windows")]
fn push_python_script_dirs(dirs: &mut Vec<PathBuf>, python_root: &Path) {
    let Ok(installations) = std::fs::read_dir(python_root) else {
        return;
    };
    for installation in installations.flatten() {
        let scripts = installation.path().join("Scripts");
        if scripts.is_dir() {
            push_unique_path(dirs, scripts);
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
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

fn push_unique_location(locations: &mut Vec<SearchLocation>, path: PathBuf) {
    if !locations.iter().any(
        |location| matches!(location, SearchLocation::Directory(existing) if existing == &path),
    ) {
        locations.push(SearchLocation::Directory(path));
    }
}

#[cfg(target_os = "windows")]
fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

fn version_arg(tool: &str) -> &'static str {
    match tool {
        "yt-dlp" => "--version",
        _ => "-version",
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "windows")]
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn finds_exe_extension_in_windows_directory() {
        let dir = crate::test_support::unique_dir("media-tools-windows-exe");
        let executable = dir.join("ffmpeg.exe");
        std::fs::write(&executable, []).unwrap();

        assert_eq!(find_in_dir(&dir, "ffmpeg"), Some(executable));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_extensions_always_include_exe() {
        assert!(
            windows_executable_extensions()
                .iter()
                .any(|extension| extension.eq_ignore_ascii_case(".exe"))
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn resolves_executable_from_windows_path() {
        let cargo = resolve("cargo").expect("cargo.exe should be resolvable from the test PATH");
        assert!(cargo.is_file());
        assert_eq!(
            cargo
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("exe")
        );
    }
}
