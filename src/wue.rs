// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Windows User Experience: autounattend.xml generation, like Rufus' wue.c.

#[derive(Clone, Default, PartialEq)]
pub struct Options {
    pub bypass_hw_checks: bool,
    pub no_online_account: bool,
    pub local_account: Option<String>,
    pub regional: bool,
    pub no_data_collection: bool,
    pub no_bitlocker: bool,
    /// Windows To Go only: keep the installed system off internal disks (SAN policy 4).
    pub wtg_no_internal: bool,
}

impl Options {
    pub fn any(&self) -> bool {
        self.bypass_hw_checks
            || self.no_online_account
            || self.local_account.is_some()
            || self.regional
            || self.no_data_collection
            || self.no_bitlocker
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Current user locale as a BCP 47 tag, e.g. "en-US".
pub fn locale() -> String {
    let from_env = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX");
    #[cfg(target_os = "macos")]
    let from_env = from_env.or_else(|| crate::drives::run("defaults", &["read", "-g", "AppleLocale"]).ok());
    #[cfg(windows)]
    let from_env = from_env.or_else(|| {
        crate::drives::run("powershell", &["-NoProfile", "-Command", "(Get-Culture).Name"]).ok()
    });
    from_env
        .map(|v| v.trim().split(['.', '@']).next().unwrap_or("").replace('_', "-"))
        .filter(|v| v.len() >= 2)
        .unwrap_or_else(|| "en-US".into())
}

fn component(name: &str, arch: &str, body: &str) -> String {
    format!(
        "    <component name=\"{name}\" processorArchitecture=\"{arch}\" language=\"neutral\" \
         xmlns:wcm=\"http://schemas.microsoft.com/WMIConfig/2002/State\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         publicKeyToken=\"31bf3856ad364e35\" versionScope=\"nonSxS\">\n{body}    </component>\n"
    )
}

fn commands(tag: &str, inner: &str, cmds: &[String]) -> String {
    let mut s = format!("      <{tag}>\n");
    for (i, c) in cmds.iter().enumerate() {
        s += &format!(
            "        <{inner} wcm:action=\"add\">\n          <Order>{}</Order>\n          <{}>{}</{}>\n        </{inner}>\n",
            i + 1,
            if inner == "SynchronousCommand" { "CommandLine" } else { "Path" },
            escape(c),
            if inner == "SynchronousCommand" { "CommandLine" } else { "Path" },
        );
    }
    s + &format!("      </{tag}>\n")
}

/// `arch` is the unattend processor architecture: "amd64", "x86" or "arm64".
pub fn unattend_xml(o: &Options, arch: &str) -> String {
    let mut x = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<unattend xmlns=\"urn:schemas-microsoft-com:unattend\">\n",
    );
    if o.bypass_hw_checks {
        let cmds: Vec<String> = ["BypassTPMCheck", "BypassSecureBootCheck", "BypassRAMCheck"]
            .iter()
            .map(|v| format!("reg add HKLM\\SYSTEM\\Setup\\LabConfig /v {v} /t REG_DWORD /d 1 /f"))
            .collect();
        let body = "      <UserData>\n        <ProductKey>\n          <Key />\n        </ProductKey>\n      </UserData>\n"
            .to_string()
            + &commands("RunSynchronous", "RunSynchronousCommand", &cmds);
        x += "  <settings pass=\"windowsPE\">\n";
        x += &component("Microsoft-Windows-Setup", arch, &body);
        x += "  </settings>\n";
    }

    let mut special = Vec::new();
    if o.no_online_account {
        special.push("reg add HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\OOBE /v BypassNRO /t REG_DWORD /d 1 /f".into());
    }
    if o.no_bitlocker {
        special.push("reg add HKLM\\SYSTEM\\CurrentControlSet\\Control\\BitLocker /v PreventDeviceEncryption /t REG_DWORD /d 1 /f".into());
    }
    if !special.is_empty() {
        x += "  <settings pass=\"specialize\">\n";
        x += &component("Microsoft-Windows-Deployment", arch, &commands("RunSynchronous", "RunSynchronousCommand", &special));
        x += "  </settings>\n";
    }

    if o.regional || o.no_data_collection || o.local_account.is_some() {
        x += "  <settings pass=\"oobeSystem\">\n";
        if o.regional {
            let l = escape(&locale());
            x += &component(
                "Microsoft-Windows-International-Core",
                arch,
                &format!(
                    "      <InputLocale>{l}</InputLocale>\n      <SystemLocale>{l}</SystemLocale>\n      \
                     <UILanguage>{l}</UILanguage>\n      <UserLocale>{l}</UserLocale>\n"
                ),
            );
        }
        let mut body = String::from("      <OOBE>\n");
        if o.no_data_collection {
            body += "        <ProtectYourPC>3</ProtectYourPC>\n";
        }
        if o.local_account.is_some() {
            body += "        <HideOnlineAccountScreens>true</HideOnlineAccountScreens>\n";
        }
        body += "      </OOBE>\n";
        if let Some(user) = &o.local_account {
            let u = escape(user);
            body += &format!(
                "      <UserAccounts>\n        <LocalAccounts>\n          <LocalAccount wcm:action=\"add\">\n            \
                 <Name>{u}</Name>\n            <DisplayName>{u}</DisplayName>\n            <Group>Administrators;Power Users</Group>\n            \
                 <Password>\n              <Value>UABhAHMAcwB3AG8AcgBkAA==</Value>\n              <PlainText>false</PlainText>\n            \
                 </Password>\n          </LocalAccount>\n        </LocalAccounts>\n      </UserAccounts>\n"
            );
            // Rufus sets a blank password and forces a change at first logon.
            body += &commands(
                "FirstLogonCommands",
                "SynchronousCommand",
                &[format!("net user \"{user}\" /logonpasswordchg:yes"), "net accounts /maxpwage:unlimited".into()],
            );
        }
        x += &component("Microsoft-Windows-Shell-Setup", arch, &body);
        x += "  </settings>\n";
    }
    x + "</unattend>\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_contains_selected_options() {
        let o = Options {
            bypass_hw_checks: true,
            no_online_account: true,
            local_account: Some("me<&>".into()),
            regional: false,
            no_data_collection: true,
            no_bitlocker: true,
            wtg_no_internal: false,
        };
        let x = unattend_xml(&o, "amd64");
        assert!(x.contains("BypassTPMCheck") && x.contains("BypassNRO") && x.contains("PreventDeviceEncryption"));
        assert!(x.contains("<ProtectYourPC>3</ProtectYourPC>"));
        assert!(x.contains("<Name>me&lt;&amp;&gt;</Name>"));
        assert!(!x.contains("me<&>"));
        assert_eq!(x.matches("<settings").count(), x.matches("</settings>").count());
        assert!(!unattend_xml(&Options::default(), "amd64").contains("<settings"));
    }
}
