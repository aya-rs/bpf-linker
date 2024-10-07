use std::ffi::OsStr;

use clap::ValueEnum;
use target_lexicon::{
    Aarch64Architecture, Architecture, BinaryFormat, Environment, OperatingSystem,
    Riscv64Architecture, Triple, Vendor,
};

use crate::llvm::{LlvmBuildConfig, Processor, System};

#[derive(Clone)]
pub enum SupportedTriple {
    Aarch64AppleDarwin,
    Aarch64UnknownLinuxGnu,
    Aarch64UnknownLinuxMusl,
    Riscv64UnknownLinuxGnu,
    Riscv64UnknownLinuxMusl,
    X86_64AppleDarwin,
    X86_64UnknownLinuxGnu,
    X86_64UnknownLinuxMusl,
}

impl ValueEnum for SupportedTriple {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            Self::Aarch64AppleDarwin,
            Self::Aarch64UnknownLinuxGnu,
            Self::Aarch64UnknownLinuxMusl,
            Self::Riscv64UnknownLinuxGnu,
            Self::Riscv64UnknownLinuxMusl,
            Self::X86_64AppleDarwin,
            Self::X86_64UnknownLinuxGnu,
            Self::X86_64UnknownLinuxMusl,
        ]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(match self {
            Self::Aarch64AppleDarwin => clap::builder::PossibleValue::new("aarch64-apple-darwin"),
            Self::Aarch64UnknownLinuxGnu => {
                clap::builder::PossibleValue::new("aarch64-unknown-linux-gnu")
            }
            Self::Aarch64UnknownLinuxMusl => {
                clap::builder::PossibleValue::new("aarch64-unknown-linux-musl")
            }
            Self::Riscv64UnknownLinuxGnu => {
                clap::builder::PossibleValue::new("riscv64-unknown-linux-gnu")
            }
            Self::Riscv64UnknownLinuxMusl => {
                clap::builder::PossibleValue::new("riscv64-unknown-linux-musl")
            }
            Self::X86_64AppleDarwin => clap::builder::PossibleValue::new("x86_64-apple-darwin"),
            Self::X86_64UnknownLinuxGnu => {
                clap::builder::PossibleValue::new("x86_64-unknown-linux-gnu")
            }
            Self::X86_64UnknownLinuxMusl => {
                clap::builder::PossibleValue::new("x86_64-unknown-linux-musl")
            }
        })
    }
}

impl From<SupportedTriple> for Triple {
    fn from(value: SupportedTriple) -> Self {
        match value {
            SupportedTriple::Aarch64AppleDarwin => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::Darwin,
                environment: Environment::Unknown,
                binary_format: BinaryFormat::Macho,
            },
            SupportedTriple::Aarch64UnknownLinuxGnu => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Unknown,
                operating_system: OperatingSystem::Linux,
                environment: Environment::Gnu,
                binary_format: BinaryFormat::Elf,
            },
            SupportedTriple::Aarch64UnknownLinuxMusl => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Unknown,
                operating_system: OperatingSystem::Linux,
                environment: Environment::Musl,
                binary_format: BinaryFormat::Elf,
            },
            SupportedTriple::Riscv64UnknownLinuxGnu => Triple {
                architecture: Architecture::Riscv64(Riscv64Architecture::Riscv64gc),
                vendor: Vendor::Unknown,
                operating_system: OperatingSystem::Linux,
                environment: Environment::Musl,
                binary_format: BinaryFormat::Elf,
            },
            SupportedTriple::Riscv64UnknownLinuxMusl => Triple {
                architecture: Architecture::Riscv64(Riscv64Architecture::Riscv64gc),
                vendor: Vendor::Unknown,
                operating_system: OperatingSystem::Linux,
                environment: Environment::Musl,
                binary_format: BinaryFormat::Elf,
            },
            SupportedTriple::X86_64AppleDarwin => Triple {
                architecture: Architecture::X86_64,
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::Darwin,
                environment: Environment::Unknown,
                binary_format: BinaryFormat::Macho,
            },
            SupportedTriple::X86_64UnknownLinuxGnu => Triple {
                architecture: Architecture::X86_64,
                vendor: Vendor::Unknown,
                operating_system: OperatingSystem::Linux,
                environment: Environment::Gnu,
                binary_format: BinaryFormat::Elf,
            },
            SupportedTriple::X86_64UnknownLinuxMusl => Triple {
                architecture: Architecture::X86_64,
                vendor: Vendor::Unknown,
                operating_system: OperatingSystem::Linux,
                environment: Environment::Musl,
                binary_format: BinaryFormat::Elf,
            },
        }
    }
}

pub trait TripleExt {
    fn containerized_build(&self) -> bool;
    fn container_image(&self) -> Option<(String, String)>;
    fn llvm_build_config(&self, install_prefix: &OsStr) -> Option<LlvmBuildConfig>;
    fn is_cross(&self) -> bool;
}

impl TripleExt for Triple {
    fn containerized_build(&self) -> bool {
        let Triple {
            operating_system, ..
        } = self;
        *operating_system == OperatingSystem::Linux
    }

    fn container_image(&self) -> Option<(String, String)> {
        let prefix = if self.is_cross() { "cross" } else { "native" };
        if self.containerized_build() {
            let tag = format!("{prefix}-{self}");
            let full_tag = format!("ghcr.io/aya-rs/bpf-linker/{tag}");
            let dockerfile = format!("docker/Dockerfile.{tag}");
            Some((full_tag, dockerfile))
        } else {
            None
        }
    }

    fn llvm_build_config(&self, install_prefix: &OsStr) -> Option<LlvmBuildConfig> {
        let Triple {
            architecture,
            operating_system,
            environment,
            ..
        } = self;
        let install_prefix = install_prefix.to_owned();

        match (architecture, operating_system, environment) {
            (Architecture::Aarch64(_), OperatingSystem::Darwin, Environment::Unknown) => {
                Some(LlvmBuildConfig {
                    c_compiler: "clang".to_owned(),
                    cxx_compiler: "clang++".to_owned(),
                    cxxflags: None,
                    ldflags: None,
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Darwin,
                    processor: Processor::Aarch64,
                    target_triple: "aarch64-apple-darwin".to_owned(),
                })
            }
            (Architecture::Aarch64(_), OperatingSystem::Linux, Environment::Gnu) => {
                Some(LlvmBuildConfig {
                    c_compiler: if self.is_cross() {
                        "aarch64-linux-gnu-clang".to_owned()
                    } else {
                        "clang".to_owned()
                    },
                    cxx_compiler: if self.is_cross() {
                        "aarch64-linux-gnu-clang++".to_owned()
                    } else {
                        "clang".to_owned()
                    },
                    cxxflags: None,
                    ldflags: None,
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Linux,
                    processor: Processor::Aarch64,
                    target_triple: "aarch64-linux-gnu".to_owned(),
                })
            }
            (Architecture::Aarch64(_), OperatingSystem::Linux, Environment::Musl) => {
                // Gentoo's crossdev doesn't work with triples not containing
                // the `gentoo` vendor.
                Some(LlvmBuildConfig {
                    c_compiler: if self.is_cross() {
                        "aarch64-gentoo-linux-musl-clang".to_owned()
                    } else {
                        "clang".to_owned()
                    },
                    cxx_compiler: if self.is_cross() {
                        "aarch64-gentoo-linux-musl-clang++".to_owned()
                    } else {
                        "clang++".to_owned()
                    },
                    cxxflags: Some("-stdlib=libc++".to_owned()),
                    ldflags: Some(
                        "-rtlib=compiler-rt -unwindlib=libunwind -lc++ -lc++abi".to_owned(),
                    ),
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Linux,
                    processor: Processor::Aarch64,
                    target_triple: "aarch64-gentoo-linux-musl".to_owned(),
                })
            }
            (Architecture::Riscv64(_), OperatingSystem::Linux, Environment::Gnu) => {
                Some(LlvmBuildConfig {
                    c_compiler: if self.is_cross() {
                        "riscv64-linux-gnu-clang".to_owned()
                    } else {
                        "clang".to_owned()
                    },
                    cxx_compiler: if self.is_cross() {
                        "riscv64-linux-gnu-clang++".to_owned()
                    } else {
                        "clang++".to_owned()
                    },
                    cxxflags: None,
                    ldflags: None,
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Linux,
                    processor: Processor::Riscv64,
                    target_triple: "riscv64-linux-gnu".to_owned(),
                })
            }
            (Architecture::Riscv64(_), OperatingSystem::Linux, Environment::Musl) => {
                // NOTE(vadorovsky): Gentoo's crossdev doesn't work with
                // triples not containing the `gentoo` vendor.
                Some(LlvmBuildConfig {
                    c_compiler: if self.is_cross() {
                        "riscv64-gentoo-linux-musl-clang".to_owned()
                    } else {
                        "clang".to_owned()
                    },
                    cxx_compiler: if self.is_cross() {
                        "riscv64-gentoo-linux-musl-clang++".to_owned()
                    } else {
                        "clang++".to_owned()
                    },
                    cxxflags: None,
                    ldflags: None,
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Linux,
                    processor: Processor::Riscv64,
                    target_triple: "riscv64-gentoo-linux-musl".to_owned(),
                })
            }
            (Architecture::X86_64, OperatingSystem::Darwin, Environment::Unknown) => {
                Some(LlvmBuildConfig {
                    c_compiler: "clang".to_owned(),
                    cxx_compiler: "clang++".to_owned(),
                    cxxflags: None,
                    ldflags: None,
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Darwin,
                    processor: Processor::X86_64,
                    target_triple: "x86_64-apple-darwin".to_owned(),
                })
            }
            (Architecture::X86_64, OperatingSystem::Linux, Environment::Gnu) => {
                Some(LlvmBuildConfig {
                    c_compiler: if self.is_cross() {
                        "x86_64-linux-gnu-clang".to_owned()
                    } else {
                        "clang".to_owned()
                    },
                    cxx_compiler: if self.is_cross() {
                        "x86_64-linux-gnu-clang++".to_owned()
                    } else {
                        "clang++".to_owned()
                    },
                    cxxflags: None,
                    ldflags: None,
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Linux,
                    processor: Processor::X86_64,
                    target_triple: "x86_64-linux-gnu".to_owned(),
                })
            }
            (Architecture::X86_64, OperatingSystem::Linux, Environment::Musl) => {
                // NOTE(vadorovsky): Gentoo's crossdev doesn't work with
                // triples not containing the `gentoo` vendor.
                Some(LlvmBuildConfig {
                    c_compiler: if self.is_cross() {
                        "x86_64-gentoo-linux-musl-clang".to_owned()
                    } else {
                        "clang".to_owned()
                    },
                    cxx_compiler: if self.is_cross() {
                        "x86_64-gentoo-linux-musl-clang++".to_owned()
                    } else {
                        "clang++".to_owned()
                    },
                    cxxflags: None,
                    ldflags: None,
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Linux,
                    processor: Processor::X86_64,
                    target_triple: "x86_64-gentoo-linux-musl".to_owned(),
                })
            }
            (_, _, _) => None,
        }
    }

    fn is_cross(&self) -> bool {
        self.architecture != target_lexicon::HOST.architecture
    }
}
