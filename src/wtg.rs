// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Windows To Go: install.wim/esd editions, and (on Windows) applying one
//! to the drive with DISM + bcdboot, as Rufus does.

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
    let mut hdr = [0u8; 0xD0];
    read_exact_at(iso, &wim, 0, &mut hdr)?;
    if &hdr[..8] != b"MSWIM\0\0\0" {
        return Err("install.wim is not a WIM file".into());
    }
    // XML data resource header at 0x48: 7-byte size + flags, then offset.
    let size = u64::from_le_bytes([
        hdr[0x48], hdr[0x49], hdr[0x4A], hdr[0x4B], hdr[0x4C], hdr[0x4D], hdr[0x4E], 0,
    ]);
    let offset = u64::from_le_bytes(hdr[0x50..0x58].try_into().unwrap());
    if size == 0 || size > 64 << 20 {
        return Err("WIM XML metadata missing or too large".into());
    }
    let mut xml = vec![0u8; size as usize];
    read_exact_at(iso, &wim, offset, &mut xml)?;
    Ok(parse_editions(&xml))
}

fn read_exact_at(iso: &Iso, e: &Entry, mut off: u64, mut buf: &mut [u8]) -> Result<(), String> {
    while !buf.is_empty() {
        let n = iso.read_at(e, off, buf).map_err(|err| err.to_string())?;
        if n == 0 {
            return Err("unexpected end of install.wim".into());
        }
        off += n as u64;
        buf = &mut buf[n..];
    }
    Ok(())
}

fn parse_editions(utf16: &[u8]) -> Vec<(u32, String)> {
    let units: Vec<u16> = utf16
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
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
mod tests {
    use super::*;

    #[test]
    fn parses_wim_xml() {
        let xml = "\u{feff}<WIM><TOTALBYTES>1</TOTALBYTES><IMAGE INDEX=\"1\"><NAME>Windows 11 Home</NAME></IMAGE>\
                   <IMAGE INDEX=\"6\"><NAME>Windows 11 Pro</NAME><DISPLAYNAME>Windows 11 Pro (display)</DISPLAYNAME></IMAGE></WIM>";
        let bytes: Vec<u8> = xml.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(
            parse_editions(&bytes),
            vec![
                (1, "Windows 11 Home".into()),
                (6, "Windows 11 Pro (display)".into())
            ]
        );
    }
}
