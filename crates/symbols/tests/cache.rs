use wind_symbols::{image::Headers, Cache, Symbol, BUILD_SYMBOL, UPDATE_SYMBOL};

fn put16(b: &mut [u8], at: usize, n: u16) {
    b[at..at + 2].copy_from_slice(&n.to_le_bytes());
}
fn put32(b: &mut [u8], at: usize, n: u32) {
    b[at..at + 4].copy_from_slice(&n.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut b = vec![0; 0x600];
    b[..2].copy_from_slice(b"MZ");
    put32(&mut b, 0x3c, 0x80);
    b[0x80..0x84].copy_from_slice(b"PE\0\0");
    put16(&mut b, 0x84, 0x8664);
    put16(&mut b, 0x86, 2);
    put32(&mut b, 0x88, 123);
    put16(&mut b, 0x94, 240);
    let o = 0x98;
    put16(&mut b, o, 0x20b);
    put32(&mut b, o + 56, 0x3000);
    put32(&mut b, o + 60, 0x200);
    put32(&mut b, o + 108, 16);
    put32(&mut b, o + 160, 0x2000);
    put32(&mut b, o + 164, 28);
    let s = o + 240;
    put32(&mut b, s + 8, 0x100);
    put32(&mut b, s + 12, 0x1000);
    put32(&mut b, s + 16, 0x200);
    put32(&mut b, s + 20, 0x200);
    put32(&mut b, s + 36, 0x60000020);
    put32(&mut b, s + 48, 0x200);
    put32(&mut b, s + 52, 0x2000);
    put32(&mut b, s + 56, 0x200);
    put32(&mut b, s + 60, 0x400);
    put32(&mut b, s + 76, 0x40000040);
    put32(&mut b, 0x40c, 2);
    put32(&mut b, 0x410, 39);
    put32(&mut b, 0x414, 0x2040);
    put32(&mut b, 0x418, 0x440);
    b[0x440..0x444].copy_from_slice(b"RSDS");
    for (i, byte) in b[0x444..0x454].iter_mut().enumerate() {
        *byte = i as u8;
    }
    put32(&mut b, 0x454, 1);
    b[0x458..0x467].copy_from_slice(b"SystemTray.pdb\0");
    b
}

#[test]
fn file_and_memory_layout_produce_the_same_identity() {
    let file = fixture();
    let h = Headers::parse(&file).unwrap();
    let disk = h
        .identity(|rva, len| h.file_bytes(&file, rva, len).map(|b| b.to_vec()))
        .unwrap();
    let mut memory = vec![0u8; h.image_size as usize];
    memory[..0x200].copy_from_slice(&file[..0x200]);
    memory[0x1000..0x1200].copy_from_slice(&file[0x200..0x400]);
    memory[0x2000..0x2200].copy_from_slice(&file[0x400..0x600]);
    let loaded = h
        .identity(|rva, len| Ok(memory[rva as usize..(rva + len) as usize].to_vec()))
        .unwrap();
    assert_eq!(disk, loaded);
    assert_eq!(disk.pdb_name, "SystemTray.pdb");
    assert!(disk
        .key()
        .starts_with("030201000504070608090A0B0C0D0E0F-1-"));
}

#[test]
fn rejects_stale_identity_wrong_overload_and_non_code_addresses() {
    let file = fixture();
    let h = Headers::parse(&file).unwrap();
    let id = h
        .identity(|rva, len| h.file_bytes(&file, rva, len).map(|b| b.to_vec()))
        .unwrap();
    let good = Cache {
        schema: 1,
        module: "SystemTray.dll".into(),
        identity: id.clone(),
        build: Symbol {
            name: BUILD_SYMBOL.into(),
            rva: 0x1010,
        },
        update: Symbol {
            name: UPDATE_SYMBOL.into(),
            rva: 0x1020,
        },
    };
    good.validate("SystemTray.dll", &h, &id).unwrap();
    for change in 0..8 {
        let mut bad = good.clone();
        match change {
            0 => bad.identity.pdb_guid[0] ^= 1,
            1 => bad.identity.pdb_age += 1,
            2 => bad.identity.timestamp += 1,
            3 => bad.identity.image_size += 4096,
            4 => bad.update.name = "?UpdateCurrentData@LanguageSystemTrayIconDataModel@implementation@SystemTray@winrt@@QEAAXAEBUInputProfileData@234@@Z".into(),
            5 => bad.build.rva = 0x2010,
            6 => bad.update.rva = bad.build.rva,
            _ => bad.build.rva = u32::MAX,
        }
        assert!(
            bad.validate("SystemTray.dll", &h, &id).is_err(),
            "accepted invalid cache case {change}"
        );
    }
    assert!(good.validate("Taskbar.View.dll", &h, &id).is_err());
}

#[test]
fn truncated_and_overflowing_pe_records_are_rejected() {
    let data = fixture();
    for end in 0..0x1d8 {
        assert!(Headers::parse(&data[..end]).is_err());
    }
    let mut bad = data.clone();
    put32(&mut bad, 0x3c, u32::MAX);
    assert!(Headers::parse(&bad).is_err());
    let mut bad = data.clone();
    put32(&mut bad, 0x414, u32::MAX);
    let h = Headers::parse(&bad).unwrap();
    assert!(h
        .identity(|rva, len| h.file_bytes(&bad, rva, len).map(|b| b.to_vec()))
        .is_err());
    let h = Headers::parse(&data).unwrap();
    assert!(h.file_bytes(&data, 0x2fff, 2).is_err());
}

#[cfg(windows)]
#[test]
fn validates_identity_and_executable_page_in_the_running_process() {
    let exe = std::env::current_exe().unwrap();
    let module =
        wind_symbols::loaded::LoadedModule::find(exe.file_name().unwrap().to_str().unwrap())
            .unwrap()
            .unwrap();
    let (_, disk) = wind_symbols::image::inspect_file(&exe).unwrap();
    assert_eq!(module.identity, disk);
    let address =
        validates_identity_and_executable_page_in_the_running_process as *const () as usize;
    assert_eq!(
        module
            .function_address((address - module.base) as u32)
            .unwrap(),
        address
    );
    assert!(module.function_address(0).is_err());
}

struct TestDirectory(std::path::PathBuf);
impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "divvun-wind-symbol-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        // These tests create only immediate files, so no recursive deletion is needed.
        if let Ok(files) = std::fs::read_dir(&self.0) {
            for file in files.flatten() {
                let _ = std::fs::remove_file(file.path());
            }
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[cfg(feature = "resolver")]
#[test]
fn cli_boot_cache_needs_neither_pdb_files_nor_dbghelp_and_rejects_stale_data() {
    if !wind_common::os::eligible() {
        return;
    }
    let temp = TestDirectory::new();
    let image = temp.0.join("SystemTray.dll");
    std::fs::write(&image, fixture()).unwrap();
    let (headers, identity) = wind_symbols::image::inspect_file(&image).unwrap();
    let mut cache = Cache {
        schema: 1,
        module: "SystemTray.dll".into(),
        identity,
        build: Symbol {
            name: BUILD_SYMBOL.into(),
            rva: 0x1010,
        },
        update: Symbol {
            name: UPDATE_SYMBOL.into(),
            rva: 0x1020,
        },
    };
    let cache_file = wind_symbols::write_cache(&temp.0, &cache).unwrap();
    // Also exercises atomic replacement of a cache that already exists on Windows.
    wind_symbols::write_cache(&temp.0, &cache).unwrap();
    let invoke = |command: &str| {
        std::process::Command::new(env!("CARGO_BIN_EXE_divvun-wind-symbols"))
            .arg(command)
            .arg("--image")
            .arg(&image)
            .arg("--cache-dir")
            .arg(&temp.0)
            .arg("--offline")
            .arg("--dbghelp")
            .arg(temp.0.join("missing-dbghelp.dll"))
            .output()
            .unwrap()
    };
    for command in ["prepare", "verify"] {
        let output = invoke(command);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["reused"], true);
    }
    // Poison the contents at the correct filename: lookup identity alone is insufficient.
    cache.identity.pdb_age += 1;
    std::fs::write(cache_file, serde_json::to_vec(&cache).unwrap()).unwrap();
    let output = invoke("verify");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("identity"));
    assert!(!headers.executable(0x2010));
}
