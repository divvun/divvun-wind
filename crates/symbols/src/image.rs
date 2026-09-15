//! Bounded PE32+ header / CodeView parsing, shared by the file resolver and DLL.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub timestamp: u32,
    pub image_size: u32,
    /// Raw GUID bytes from the image's RSDS record (Windows GUID byte order).
    pub pdb_guid: [u8; 16],
    pub pdb_age: u32,
    pub pdb_name: String,
}

impl Identity {
    pub fn key(&self) -> String {
        let g = self.pdb_guid;
        let guid = format!(
            "{:08X}{:04X}{:04X}{}",
            u32::from_le_bytes(g[0..4].try_into().unwrap()),
            u16::from_le_bytes(g[4..6].try_into().unwrap()),
            u16::from_le_bytes(g[6..8].try_into().unwrap()),
            g[8..]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<String>()
        );
        format!(
            "{guid}-{:X}-{:08X}-{:X}",
            self.pdb_age, self.timestamp, self.image_size
        )
    }
}

#[derive(Clone, Debug)]
pub struct Section {
    pub rva: u32,
    pub size: u32,
    raw_offset: u32,
    raw_size: u32,
    executable: bool,
}

#[derive(Clone, Debug)]
pub struct Headers {
    pub image_size: u32,
    timestamp: u32,
    header_size: u32,
    debug_rva: u32,
    debug_size: u32,
    pub sections: Vec<Section>,
}

fn bytes(data: &[u8], offset: usize, size: usize) -> Result<&[u8], String> {
    data.get(offset..offset.checked_add(size).ok_or("PE range overflow")?)
        .ok_or_else(|| "truncated PE data".into())
}
fn u16_at(data: &[u8], offset: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        bytes(data, offset, 2)?.try_into().unwrap(),
    ))
}
fn u32_at(data: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        bytes(data, offset, 4)?.try_into().unwrap(),
    ))
}

impl Headers {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if bytes(data, 0, 2)? != b"MZ" {
            return Err("not a PE image".into());
        }
        let pe = u32_at(data, 0x3c)? as usize;
        if pe > 1024 * 1024 || bytes(data, pe, 4)? != b"PE\0\0" {
            return Err("invalid PE signature".into());
        }
        if u16_at(data, pe + 4)? != 0x8664 {
            return Err("only AMD64 images are supported".into());
        }
        let count = u16_at(data, pe + 6)? as usize;
        let optional_size = u16_at(data, pe + 20)? as usize;
        let opt = pe + 24;
        if count == 0 || count > 96 || optional_size < 168 || u16_at(data, opt)? != 0x20b {
            return Err("invalid PE32+ headers".into());
        }
        if u32_at(data, opt + 108)? < 7 {
            return Err("PE has no debug directory".into());
        }
        let image_size = u32_at(data, opt + 56)?;
        let header_size = u32_at(data, opt + 60)?;
        if image_size == 0 || header_size > image_size {
            return Err("invalid image size".into());
        }
        let mut sections = Vec::new();
        for n in 0..count {
            let s = opt + optional_size + n * 40;
            let rva = u32_at(data, s + 12)?;
            let size = u32_at(data, s + 8)?;
            if rva
                .checked_add(size)
                .filter(|&end| end <= image_size)
                .is_none()
            {
                return Err("section outside image".into());
            }
            sections.push(Section {
                rva,
                size,
                raw_offset: u32_at(data, s + 20)?,
                raw_size: u32_at(data, s + 16)?,
                executable: u32_at(data, s + 36)? & 0x20000000 != 0,
            });
        }
        Ok(Self {
            image_size,
            header_size,
            timestamp: u32_at(data, pe + 8)?,
            debug_rva: u32_at(data, opt + 160)?,
            debug_size: u32_at(data, opt + 164)?,
            sections,
        })
    }

    pub fn executable(&self, rva: u32) -> bool {
        self.sections
            .iter()
            .any(|s| s.executable && rva >= s.rva && rva - s.rva < s.size)
    }

    pub fn file_bytes<'a>(&self, data: &'a [u8], rva: u32, len: u32) -> Result<&'a [u8], String> {
        self.check_range(rva, len)?;
        if rva
            .checked_add(len)
            .filter(|&end| end <= self.header_size)
            .is_some()
        {
            return bytes(data, rva as usize, len as usize);
        }
        for s in &self.sections {
            if let Some(delta) = rva.checked_sub(s.rva) {
                if delta
                    .checked_add(len)
                    .filter(|&end| end <= s.raw_size)
                    .is_some()
                {
                    let offset = s
                        .raw_offset
                        .checked_add(delta)
                        .ok_or("file offset overflow")?;
                    return bytes(data, offset as usize, len as usize);
                }
            }
        }
        Err("RVA has no file data".into())
    }

    pub fn check_range(&self, rva: u32, len: u32) -> Result<(), String> {
        if rva
            .checked_add(len)
            .filter(|&end| end <= self.image_size)
            .is_none()
        {
            return Err("RVA outside image".into());
        }
        Ok(())
    }

    pub fn identity(
        &self,
        mut read: impl FnMut(u32, u32) -> Result<Vec<u8>, String>,
    ) -> Result<Identity, String> {
        if self.debug_rva == 0
            || self.debug_size == 0
            || self.debug_size > 28 * 256
            || !self.debug_size.is_multiple_of(28)
        {
            return Err("invalid or absent debug directory".into());
        }
        self.check_range(self.debug_rva, self.debug_size)?;
        let debug = read(self.debug_rva, self.debug_size)?;
        for entry in debug.as_chunks::<28>().0 {
            if u32_at(entry, 12)? != 2 {
                continue;
            } // IMAGE_DEBUG_TYPE_CODEVIEW
            let size = u32_at(entry, 16)?;
            let rva = u32_at(entry, 20)?;
            if !(25..=4096).contains(&size) {
                continue;
            }
            self.check_range(rva, size)?;
            let cv = read(rva, size)?;
            if bytes(&cv, 0, 4)? != b"RSDS" {
                continue;
            }
            let name_bytes = bytes(&cv, 24, cv.len().saturating_sub(24))?;
            let end = name_bytes
                .iter()
                .position(|&b| b == 0)
                .ok_or("unterminated PDB name")?;
            let name = std::str::from_utf8(&name_bytes[..end]).map_err(|_| "invalid PDB name")?;
            let pdb_name = name
                .rsplit(['\\', '/'])
                .next()
                .ok_or("empty PDB name")?
                .to_string();
            if pdb_name.is_empty() {
                return Err("empty PDB name".into());
            }
            return Ok(Identity {
                timestamp: self.timestamp,
                image_size: self.image_size,
                pdb_guid: bytes(&cv, 4, 16)?.try_into().unwrap(),
                pdb_age: u32_at(&cv, 20)?,
                pdb_name,
            });
        }
        Err("no RSDS identity in image".into())
    }
}

pub fn inspect_file(path: &std::path::Path) -> Result<(Headers, Identity), String> {
    let data = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let headers = Headers::parse(&data)?;
    let id =
        headers.identity(|rva, len| headers.file_bytes(&data, rva, len).map(|b| b.to_vec()))?;
    Ok((headers, id))
}
