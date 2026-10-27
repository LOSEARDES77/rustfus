// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Windows To Go: install.wim/esd editions, and (on Windows) applying one
//! to the drive with DISM + bcdboot, as Rufus does.

use crate::bytes::get64;
use crate::iso::{Entry, Iso};

#[cfg_attr(not(windows), allow(dead_code))]
/// Rufus' "Prevent Windows To Go from accessing internal disks" policy.
pub const SAN_POLICY_XML: &str = r#"<?xml version='1.0' encoding='utf-8' standalone='yes'?>
<unattend xmlns="urn:schemas-microsoft-com:unattend">
  <settings pass="offlineServicing">
    <component name="Microsoft-Windows-PartitionManager" processorArchitecture="ARCH" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS" xmlns:wcm="http://schemas.microsoft.com/WMIConfig/2002/State" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
      <SanPolicy>4</SanPolicy>
    </component>
  </settings>
</unattend>
"#;

pub fn wim_entry(iso: &Iso) -> Option<Entry> {
    iso.entries().ok()?.into_iter().find(|e| {
        let p = e.path.to_ascii_lowercase();
        p == "sources/install.wim" || p == "sources/install.esd"
    })
}

/// (index, name) of every image in the WIM, from its XML metadata resource.
pub fn editions(iso: &Iso) -> Result<Vec<(u32, String)>, String> {
    let wim = wim_entry(iso).ok_or("no sources/install.wim or install.esd in this image")?;
    let read = |off, buf: &mut [u8]| match iso.read_at(&wim, off, buf) {
        Ok(n) if n == buf.len() => Ok(()),
        Ok(_) => Err("unexpected end of install.wim".to_string()),
        Err(e) => Err(e.to_string()),
    };
    let mut hdr = [0u8; 0xd0];
    read(0, &mut hdr)?;
    if &hdr[..8] != b"MSWIM\0\0\0" {
        return Err("install.wim is not a WIM file".into());
    }
    // XML data resource header at 0x48: 7-byte size + flags, then offset.
    let size = get64(&hdr, 0x48) & 0x00ff_ffff_ffff_ffff;
    let offset = get64(&hdr, 0x50);
    if size == 0 || size > 64 << 20 {
        return Err("WIM XML metadata missing or too large".into());
    }
    let mut xml = vec![0u8; size as usize];
    read(offset, &mut xml)?;
    Ok(parse_editions(&xml))
}

fn parse_editions(utf16: &[u8]) -> Vec<(u32, String)> {
    let units: Vec<u16> = utf16
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let xml = String::from_utf16_lossy(&units);
    let tag = |s: &str, t: &str| -> Option<String> {
        let start = s.find(&format!("<{t}>"))? + t.len() + 2;
        let end = s[start..].find(&format!("</{t}>"))?;
        Some(s[start..start + end].trim().to_string())
    };
    xml.split("<IMAGE INDEX=\"")
        .skip(1)
        .filter_map(|img| {
            let index = img.split('"').next()?.parse().ok()?;
            let body = &img[..img.find("</IMAGE>").unwrap_or(img.len())];
            let name = tag(body, "DISPLAYNAME").or_else(|| tag(body, "NAME"))?;
            Some((index, name))
        })
        .collect()
}

#[cfg(test)]
#[path = "../tests/unit/wtg.rs"]
mod tests;
