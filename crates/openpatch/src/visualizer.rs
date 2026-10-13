use std::{fs::OpenOptions, io::{self, Write}, path::Path};

pub fn export(path: &Path) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(include_bytes!("../../../assets/visualizer.html"))
}
