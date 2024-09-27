use std::ffi::OsString;

use crate::cargo::{run_cargo, CargoArgs};

pub fn check(args: CargoArgs) -> anyhow::Result<()> {
    run_cargo(args, OsString::from("check"), &[])
}
