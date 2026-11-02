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
    Ok(parse_editions(&metadata(iso)?))
}

/// Windows 11 installation media, identified by image metadata rather than the
/// ISO's filename or volume label. Windows Server also uses build 22000+.
pub fn is_windows_11(iso: &Iso) -> bool {
    metadata(iso).is_ok_and(|xml| windows_11_metadata(&xml))
}

fn metadata(iso: &Iso) -> Result<Vec<u8>, String> {
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
    let resource = get64(&hdr, 0x48);
    let size = resource & 0x00ff_ffff_ffff_ffff;
    let offset = get64(&hdr, 0x50);
    if size == 0
        || size > 64 << 20
        || offset < hdr.len() as u64
        || offset.checked_add(size).is_none_or(|end| end > wim.size)
        || (resource >> 56) & 0x04 != 0
    {
        return Err("WIM XML metadata missing or too large".into());
    }
    let mut xml = vec![0u8; size as usize];
    read(offset, &mut xml)?;
    Ok(xml)
}

fn metadata_field<'a>(parent: roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    parent
        .children()
        .find(|node| node.has_tag_name(name))
        .and_then(|node| node.text())
        .map(str::trim)
}

fn windows_11_metadata(utf16: &[u8]) -> bool {
    if !utf16.len().is_multiple_of(2) {
        return false;
    }
    let (pairs, _) = utf16.as_chunks::<2>();
    let units: Vec<u16> = pairs
        .iter()
        .map(|bytes| u16::from_le_bytes(*bytes))
        .collect();
    let Ok(xml) = String::from_utf16(&units) else {
        return false;
    };
    let Ok(document) = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')) else {
        return false;
    };
    if !document.root_element().has_tag_name("WIM") {
        return false;
    }
    document
        .root_element()
        .children()
        .filter(|node| node.has_tag_name("IMAGE"))
        .any(|image| {
            let windows = image.children().find(|node| node.has_tag_name("WINDOWS"));
            let server = windows.is_some_and(|node| {
                metadata_field(node, "INSTALLATIONTYPE")
                    .is_some_and(|s| s.to_ascii_lowercase().contains("server"))
                    || metadata_field(node, "PRODUCTTYPE")
                        .is_some_and(|s| !s.eq_ignore_ascii_case("WinNT"))
            });
            if server {
                return false;
            }
            let named_windows_11 = ["NAME", "DISPLAYNAME"].into_iter().any(|name| {
                let Some(name) = metadata_field(image, name) else {
                    return false;
                };
                let lower = name.to_ascii_lowercase();
                let mut words = lower.split_whitespace();
                words.next() == Some("windows") && words.next() == Some("11")
            });
            named_windows_11
                || windows.is_some_and(|node| {
                    metadata_field(node, "INSTALLATIONTYPE")
                        .is_some_and(|s| s.eq_ignore_ascii_case("Client"))
                        && node
                            .children()
                            .find(|n| n.has_tag_name("VERSION"))
                            .is_some_and(|version| {
                                metadata_field(version, "MAJOR") == Some("10")
                                    && metadata_field(version, "BUILD")
                                        .and_then(|s| s.parse::<u32>().ok())
                                        .is_some_and(|build| build >= 22000)
                            })
                })
        })
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
