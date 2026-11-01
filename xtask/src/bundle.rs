//! `cargo xtask bundle`: packs the Phase 0 spike programs into a zip that a
//! non-technical owner can unzip, run, and send the report back from.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

/// The programs the run scripts execute, in order. The scripts hard-code the same list; a test
/// checks they agree.
pub const PROGRAMS: &[&str] = &[
    "spike-gpu",
    "spike-wasmtime",
    "spike-mesher",
    "spike-redb",
    "spike-circuit",
    "spike-quinn",
];

/// What to bundle and for which platform.
pub struct Spec<'a> {
    /// Directory holding the built programs (without extension on Unix, `.exe` on Windows).
    pub bin_dir: &'a Path,
    pub out_dir: &'a Path,
    /// `std::env::consts::OS` value: "windows", "macos" or "linux".
    pub os: &'a str,
    /// Shown in the zip name, e.g. "x86_64", "aarch64" or "universal".
    pub arch_label: &'a str,
}

struct Script {
    name: &'static str,
    body: &'static str,
    readme: &'static str,
    executable: bool,
    crlf: bool,
}

fn script_for(os: &str) -> Result<Script> {
    Ok(match os {
        "windows" => Script {
            name: "run.bat",
            body: include_str!("../bundle/run.bat"),
            readme: include_str!("../bundle/README-windows.txt"),
            executable: false,
            crlf: true,
        },
        "macos" => Script {
            name: "run.command",
            body: include_str!("../bundle/run.command"),
            readme: include_str!("../bundle/README-macos.txt"),
            executable: true,
            crlf: false,
        },
        "linux" => Script {
            name: "run.sh",
            body: include_str!("../bundle/run.sh"),
            readme: include_str!("../bundle/README-linux.txt"),
            executable: true,
            crlf: false,
        },
        other => bail!("no bundle for operating system {other:?}"),
    })
}

/// Builds `<out_dir>/voxel-test-<os>-<arch_label>.zip` and returns its path.
pub fn build(spec: &Spec) -> Result<PathBuf> {
    let script = script_for(spec.os)?;
    let suffix = if spec.os == "windows" { ".exe" } else { "" };

    // Read everything before creating the output, so a missing program leaves no partial zip.
    let mut programs = Vec::new();
    for name in PROGRAMS {
        let path = spec.bin_dir.join(format!("{name}{suffix}"));
        let bytes = fs::read(&path)
            .with_context(|| format!("missing program {} (build it first)", path.display()))?;
        programs.push((format!("{name}{suffix}"), bytes));
    }

    fs::create_dir_all(spec.out_dir)?;
    let zip_path = spec
        .out_dir
        .join(format!("voxel-test-{}-{}.zip", spec.os, spec.arch_label));
    let mut zip = ZipWriter::new(File::create(&zip_path)?);
    let plain = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let exec = plain.unix_permissions(0o755);

    for (name, bytes) in &programs {
        zip.start_file(format!("voxel-test/{name}"), exec)?;
        zip.write_all(bytes)?;
    }
    let text = |s: &str| {
        if script.crlf {
            s.replace("\r\n", "\n").replace('\n', "\r\n")
        } else {
            s.to_string()
        }
    };
    zip.start_file(
        format!("voxel-test/{}", script.name),
        if script.executable { exec } else { plain },
    )?;
    zip.write_all(text(script.body).as_bytes())?;
    zip.start_file("voxel-test/README.txt", plain)?;
    zip.write_all(text(script.readme).as_bytes())?;
    zip.start_file("voxel-test/BUNDLE-INFO.txt", plain)?;
    zip.write_all(text(&info(spec)).as_bytes())?;
    zip.finish()?;
    Ok(zip_path)
}

fn info(spec: &Spec) -> String {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    };
    format!(
        "Voxel game test bundle\nplatform: {} {}\ncommit: {}\ncommitted: {}\nprograms: {}\n",
        spec.os,
        spec.arch_label,
        git(&["rev-parse", "HEAD"]),
        git(&["log", "-1", "--format=%cI"]),
        PROGRAMS.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-bundle-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn fake_programs(dir: &Path, suffix: &str) {
        for name in PROGRAMS {
            fs::write(
                dir.join(format!("{name}{suffix}")),
                format!("binary {name}"),
            )
            .expect("write");
        }
    }

    fn entries(zip: &Path) -> Vec<(String, Option<u32>, Vec<u8>)> {
        let mut archive =
            zip::ZipArchive::new(File::open(zip).expect("open zip")).expect("read zip");
        (0..archive.len())
            .map(|i| {
                let mut f = archive.by_index(i).expect("entry");
                let mut bytes = Vec::new();
                f.read_to_end(&mut bytes).expect("read entry");
                (
                    f.name().expect("utf8 name").to_string(),
                    f.unix_mode(),
                    bytes,
                )
            })
            .collect()
    }

    #[test]
    fn windows_bundle_has_exes_crlf_script_and_readme() {
        let (bin, out) = (scratch("win-bin"), scratch("win-out"));
        fake_programs(&bin, ".exe");
        let zip = build(&Spec {
            bin_dir: &bin,
            out_dir: &out,
            os: "windows",
            arch_label: "x86_64",
        })
        .expect("bundle");
        assert_eq!(
            zip.file_name().expect("name"),
            "voxel-test-windows-x86_64.zip"
        );
        let e = entries(&zip);
        for name in PROGRAMS {
            assert!(
                e.iter()
                    .any(|(n, _, _)| *n == format!("voxel-test/{name}.exe")),
                "{name}"
            );
        }
        let bat = e
            .iter()
            .find(|(n, _, _)| n == "voxel-test/run.bat")
            .expect("run.bat");
        let text = String::from_utf8(bat.2.clone()).expect("utf8");
        assert!(
            text.contains("\r\n") && !text.replace("\r\n", "").contains('\n'),
            "all CRLF"
        );
        assert!(e.iter().any(|(n, _, _)| n == "voxel-test/README.txt"));
        assert!(e.iter().any(|(n, _, b)| n == "voxel-test/BUNDLE-INFO.txt"
            && String::from_utf8_lossy(b).contains("commit:")));
    }

    #[test]
    fn macos_bundle_keeps_executable_bits() {
        let (bin, out) = (scratch("mac-bin"), scratch("mac-out"));
        fake_programs(&bin, "");
        let zip = build(&Spec {
            bin_dir: &bin,
            out_dir: &out,
            os: "macos",
            arch_label: "universal",
        })
        .expect("bundle");
        for (name, mode, _) in entries(&zip) {
            let want_exec = name.contains("spike-") || name.ends_with("run.command");
            let is_exec = mode.expect("unix mode recorded") & 0o111 != 0;
            assert_eq!(is_exec, want_exec, "{name}");
        }
    }

    #[test]
    fn missing_program_is_an_error_and_leaves_no_zip() {
        let (bin, out) = (scratch("miss-bin"), scratch("miss-out"));
        fake_programs(&bin, "");
        fs::remove_file(bin.join("spike-quinn")).expect("remove");
        let err = build(&Spec {
            bin_dir: &bin,
            out_dir: &out,
            os: "linux",
            arch_label: "x86_64",
        })
        .expect_err("must fail");
        assert!(format!("{err:#}").contains("spike-quinn"), "{err:#}");
        assert!(
            fs::read_dir(&out).expect("read out").next().is_none(),
            "no partial zip"
        );
    }

    #[test]
    fn unknown_os_is_rejected() {
        let (bin, out) = (scratch("os-bin"), scratch("os-out"));
        assert!(
            build(&Spec {
                bin_dir: &bin,
                out_dir: &out,
                os: "plan9",
                arch_label: "x"
            })
            .is_err()
        );
    }

    #[test]
    fn every_script_runs_every_program() {
        for os in ["windows", "macos", "linux"] {
            let script = script_for(os).expect("script");
            for name in PROGRAMS {
                assert!(
                    script.body.contains(name),
                    "{} does not mention {name}",
                    script.name
                );
            }
        }
    }
}
