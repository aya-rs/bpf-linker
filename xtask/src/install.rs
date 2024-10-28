use std::ffi::OsString;

use crate::cargo::{run_cargo, CargoArgs};

pub fn install(args: CargoArgs) -> anyhow::Result<()> {
    run_cargo(
        args,
        OsString::from("install"),
        &[
            OsString::from("--path"),
            OsString::from("."),
            OsString::from("--root"),
            OsString::from("/root/host-cargo"),
        ],
    )
}
