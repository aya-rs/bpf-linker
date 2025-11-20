#[cfg(all(
    feature = "no-llvm-linking",
    any(feature = "deps-link-static", feature = "llvm-link-static")
))]
compile_error!(
    "`no-llvm-linking` and linking features (`deps-link-static` and
`llvm-link-static`) are mutually exclusive"
);

/// Points cargo to static library names of LLVM and dependencies needed by
/// LLVM.
///
/// That includes figuring out which libraries LLVM depends on:
///
/// - standard C++ library (GNU stdc++ or LLVM libc++),
/// - zlib (optional),
/// - zstd (optional).
///
/// Given that static libraries have no feature of pointing to dependencies,
/// we need to figure that out by looking at symbols included in LLVM.
#[cfg(all(feature = "llvm-link-static", not(feature = "no-llvm-linking")))]
fn link_llvm_static(llvm_lib_dir: &std::path::Path) -> anyhow::Result<()> {
    use std::{borrow::Cow, env, fs, process::Command};

    use anyhow::{Context as _, anyhow};
    use object::{Object as _, ObjectSymbol as _, read::archive::ArchiveFile};

    // LLVM often creates multiple symlinks, e.g. LLVM.so and LLVM-21.so, both
    // pointing to the same target.
    // To avoid duplication, track unique targets by inode, so identical
    // symlinks can be skipped.
    #[cfg(unix)]
    let mut seen_targets: std::collections::HashSet<u64> = std::collections::HashSet::new();

    // Link the library files found inside the directory.
    let mut libraries = Vec::new();
    let dir_entries = fs::read_dir(llvm_lib_dir)
        .with_context(|| format!("failed to read directory {}", llvm_lib_dir.display()))?;
    for entry in dir_entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to read entry of the directory {}",
                llvm_lib_dir.display()
            )
        })?;
        let file_name = entry.file_name();
        let file_name = file_name.into_string().map_err(|name| {
            anyhow!("archive path {} is not valid UTF-8", name.to_string_lossy())
        })?;
        if !(file_name.starts_with("libLLVM") && file_name.ends_with(".a")) {
            continue;
        }

        // Dedup by actual target file (follow symlink).
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;

            let path = entry.path();
            let meta = fs::metadata(&path)
                .with_context(|| format!("failed to get the metadata of {}", path.display()))?;
            if !seen_targets.insert(meta.ino()) {
                continue;
            }
        }

        let trimmed = file_name
            .strip_prefix("lib")
            .and_then(|name| name.strip_suffix(".a"))
            .expect("prefix and suffix were checked above")
            // `file_name` does not live long enough.
            .to_owned();
        libraries.push(trimmed);
    }
    for archive in libraries {
        println!("cargo:rustc-link-lib=static={archive}")
    }

    // Static libraries have no metadata indicating a dependency on other
    // libraries. Given that zlib and zstd might or might be not enabled in
    // different LLVM builds, check whether libLLVMSupport references their
    // symbols.
    let (mut needs_zlib, mut needs_zstd) = (false, false);
    let llvm_support = llvm_lib_dir.join("libLLVMSupport.a");
    let data = fs::read(&llvm_support)
        .with_context(|| format!("failed to read library {}", llvm_support.display()))?;
    let archive = ArchiveFile::parse(data.as_slice())
        .with_context(|| format!("failed to parse library archive {}", llvm_support.display()))?;
    'outer: for member in archive.members() {
        let member = member.with_context(|| {
            format!(
                "failed to process the member of library archive {}",
                llvm_support.display()
            )
        })?;
        let member_data = member
            .data(data.as_slice())
            .with_context(|| "failed to read member data")?;
        let obj = object::File::parse(member_data).with_context(|| {
            format!(
                "count not parse object file {}",
                String::from_utf8_lossy(member.name())
            )
        })?;
        for symbol in obj.symbols() {
            if symbol.is_undefined() {
                let sym_name = symbol.name().with_context(|| {
                    format!(
                        "invalid symbol name in object file {}",
                        String::from_utf8_lossy(member.name())
                    )
                })?;
                if sym_name.contains("crc32") {
                    needs_zlib = true;
                } else if sym_name.contains("ZSTD") {
                    needs_zstd = true;
                }
                if needs_zlib && needs_zstd {
                    break 'outer;
                }
            }
        }
    }

    // Figure out which C++ standard library to use.
    const CXXSTDLIB: &str = "CXXSTDLIB";
    let cxxstdlib = env::var(CXXSTDLIB).map(Cow::Owned).unwrap_or_else(|_| {
        let cxxstdlib = if cfg!(target_os = "linux") {
            // Default to GNU libstdc++ on Linux. Can be overwritten through
            // `CXXSTDLIB` variable on distributions using LLVM as default
            // toolchain.
            "stdc++"
        } else if cfg!(any(target_os = "macos")) {
            // Default to LLVM libc++ on macOS, where LLVM is the default
            // toolchain.
            "c++"
        } else {
            // Fall back to GNU libstdc++ on all other platforms. Again,
            // can be overwritten through `CXXSTDLIB`.
            "stdc++"
        };
        Cow::Borrowed(cxxstdlib)
    });

    let linkage = if cfg!(feature = "deps-link-static") {
        // Unfortunately, Rust/cargo don't look up for static libraries in
        // system directories, like C compilers do, so we had to implement the
        // logic of searching for them ourselves.

        // Use C compiler to retrieve the system library paths.
        let cc = env::var("CC").unwrap_or_else(|_| "cc".to_string());
        let cc_output = Command::new(&cc)
            .arg("-print-search-dirs")
            .output()
            .with_context(|| format!("failed to run `{cc} -print-search-dirs``"))?;
        if !cc_output.status.success() {
            anyhow::bail!(
                "`{cc} -print-search-dirs` failed with status: {}",
                cc_output.status
            );
        }
        let cc_stdout = String::from_utf8(cc_output.stdout)
            .with_context(|| format!("output of `{cc} -print-search-dirs` is not valid UTF-8"))?;
        let ld_paths = cc_stdout
        .lines()
        .find_map(|line| {
            line.strip_prefix("libraries: =")
        })
        .ok_or_else(|| {
            anyhow!(
                "failed to find library paths in the output of `{cc} -print-search-dirs`: {cc_stdout}"
            )
        })?;

        // Find directories with static libraries we're interested in:
        // - C++ standard library
        // - zlib (if needed)
        // - zstd (if needed)
        let cxxstdlib_filename = format!("lib{cxxstdlib}.a");
        let zlib_filename = "libz.a";
        let zstd_filename = "libzstd.a";
        let mut sys_lib_paths = Vec::new();
        let (mut cxxstdlib_found, mut zlib_found, mut zstd_found) = (false, false, false);
        for ld_path in env::split_paths(ld_paths) {
            let mut found_any = false;
            if !cxxstdlib_found && ld_path.join(&cxxstdlib_filename).exists() {
                cxxstdlib_found = true;
                found_any = true;
            }
            if needs_zlib && !zlib_found && ld_path.join(zlib_filename).exists() {
                zlib_found = true;
                found_any = true;
            }
            if needs_zstd && !zstd_found && ld_path.join(zstd_filename).exists() {
                zstd_found = true;
                found_any = true;
            }
            if found_any {
                sys_lib_paths.push(ld_path);
            }
        }
        if !cxxstdlib_found {
            anyhow::bail!(
                "could not find {cxxstdlib_filename} in any of the following directories: {ld_paths}"
            );
        }
        if needs_zlib && !zlib_found {
            anyhow::bail!(
                "could not find {zlib_filename} in any of the following directories: {ld_paths}"
            );
        }
        if needs_zstd && !zstd_found {
            anyhow::bail!(
                "could not find {zstd_filename} in any of the following directories: {ld_paths}"
            );
        }
        for sys_lib_path in sys_lib_paths {
            let sys_lib_path = sys_lib_path
                .to_str()
                .ok_or_else(|| anyhow!("path {} is not valid UTF-8", sys_lib_path.display()))?;
            println!("cargo:rustc-link-search={sys_lib_path}");
        }

        "static"
    } else {
        "dylib"
    };

    println!("cargo:rustc-link-lib={linkage}={cxxstdlib}");
    if needs_zlib {
        println!("cargo:rustc-link-lib={linkage}=z");
    }
    if needs_zstd {
        println!("cargo:rustc-link-lib={linkage}=zstd");
    }

    Ok(())
}

#[cfg(all(not(feature = "llvm-link-static"), not(feature = "no-llvm-linking")))]
fn link_llvm_dynamic(llvm_lib_dir: &std::path::Path) -> anyhow::Result<()> {
    use std::fs;

    use anyhow::{Context as _, anyhow};

    #[cfg(target_os = "macos")]
    const DYLIB_EXT: &str = ".dylib";
    #[cfg(not(target_os = "macos"))]
    const DYLIB_EXT: &str = ".so";

    let dir_entries = fs::read_dir(llvm_lib_dir).with_context(|| {
        format!(
            "failed to read entry of the directory {}",
            llvm_lib_dir.display()
        )
    })?;
    let mut library = None;
    for entry in dir_entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to read entry of the directory {}",
                llvm_lib_dir.display()
            )
        })?;
        let file_name = entry.file_name();
        let file_name = file_name.into_string().map_err(|name| {
            anyhow!("archive path {} is not valid UTF-8", name.to_string_lossy())
        })?;
        if file_name.starts_with("libLLVM") && file_name.ends_with(DYLIB_EXT) {
            library = Some(
                file_name
                    .strip_prefix("lib")
                    .and_then(|name| name.strip_suffix(DYLIB_EXT))
                    .expect("prefix and suffix were checked above")
                    // `file_name` does not live long enough.
                    .to_owned(),
            );
        }
    }
    let library = library.ok_or_else(|| {
        anyhow!(
            "could not find dynamic libLLVM in the directory {}",
            llvm_lib_dir.display()
        )
    })?;
    println!("cargo:rustc-link-lib=dylib={library}");

    Ok(())
}

/// Points cargo to the path containing LLVM libraries and to the LLVM library
/// files.
#[cfg(not(feature = "no-llvm-linking"))]
fn link_llvm() -> anyhow::Result<()> {
    use std::{env, fs, path::Path};

    use anyhow::{Context as _, anyhow};

    // If `LLVM_PREFIX` variable is not provided, find the directory with LLVM
    // libraries by assuming it's `lib/` inside a prefix where `llvm-config`
    // lives.
    const LLVM_PREFIX: &str = "LLVM_PREFIX";
    const PATH: &str = "PATH";
    let (var_name, paths_os) = env::var_os(LLVM_PREFIX)
        .map(|mut p| {
            p.push("/bin");
            (LLVM_PREFIX, p)
        })
        .or_else(|| env::var_os(PATH).map(|p| (PATH, p)))
        .ok_or_else(|| anyhow!("neither {LLVM_PREFIX} nor {PATH} is set"))?;
    let llvm_config = env::split_paths(&paths_os)
        .find_map(|dir| {
            let candidate = Path::new(&dir).join("llvm-config");
            candidate.exists().then_some(candidate)
        })
        .with_context(|| format!("could not find llvm-config in {var_name}"))?;
    let llvm_lib_dir = llvm_config
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| {
            anyhow!(
                "llvm-config location has no parent: {}",
                llvm_config.display()
            )
        })?
        .join("lib");
    let llvm_lib_dir = fs::canonicalize(&llvm_lib_dir).with_context(|| {
        format!(
            "failed to canonicalize LLVM lib directory {}",
            llvm_lib_dir.display()
        )
    })?;
    let llvm_lib_dir_str = llvm_lib_dir
        .to_str()
        .ok_or_else(|| anyhow!("path {} is not a valid UTF-8", llvm_lib_dir.display()))?;
    println!("cargo:rustc-link-search={llvm_lib_dir_str}");

    #[cfg(feature = "llvm-link-static")]
    link_llvm_static(&llvm_lib_dir)?;

    // Dynamic linking does not require search paths at the build time.
    // libLLVM.{dylib,so} should include all dependencies as `DT_NEEDED`
    // entries, pointing to them explicitly is not needed.
    #[cfg(not(feature = "llvm-link-static"))]
    link_llvm_dynamic(&llvm_lib_dir)?;

    Ok(())
}

fn main() -> anyhow::Result<()> {
    #[cfg(not(feature = "no-llvm-linking"))]
    link_llvm()?;

    Ok(())
}
