//! `lyre init` — scaffold a new lyre project.
//!
//! Writes a `lyre.luard` declaration file (so the editor knows about the
//! built-in `File`/`Folder` classes), a `luari.json` that imports it, and a
//! starter `main.luar`. Existing files are never overwritten.

use std::path::{Path, PathBuf};

pub fn run(dir: Option<String>) -> std::io::Result<()> {
    let root = match dir {
        Some(d) => PathBuf::from(d),
        None => std::env::current_dir()?,
    };
    std::fs::create_dir_all(&root)?;

    write_if_absent(&root.join("lyre.luard"), crate::fs::LUARD_DECLARATIONS)?;
    write_if_absent(&root.join("luari.json"), LUARI_JSON)?;
    write_if_absent(&root.join("main.luar"), MAIN_LUAR)?;

    println!("lyre: project ready in {}", root.display());
    println!("      run it with:  lyre main.luar");
    Ok(())
}

fn write_if_absent(path: &Path, contents: &str) -> std::io::Result<()> {
    if path.exists() {
        println!("  skip   {} (already exists)", path.display());
    } else {
        std::fs::write(path, contents)?;
        println!("  create {}", path.display());
    }
    Ok(())
}

const LUARI_JSON: &str = r#"{
    "luard": "./lyre.luard"
}
"#;

const MAIN_LUAR: &str = r#"-- Starter lyre script. Run it with:  lyre main.luar
--
-- File and Folder are built-in classes provided by the runtime. Extend them
-- to make your own typed handles, or use them directly. Paths are relative to
-- this file: "./" is this folder, "../" is one up, ".../" is two up.

class Save extends File {}

local save = Save()
save:ClaimFile("./save.txt")

if not save:Exists() then
    print("creating save file")
    save:Create()
end

save:Write("hello from lyre\n")
save:Append("second line\n")

print("save contents:")
print(save:Read())

save:Unclaim()

local here = Folder()
here:Open("./")
print("files next to this script:")
for _, name in ipairs(here:ListFiles()) do
    print("  " .. name)
end
"#;
