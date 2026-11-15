//! Windows taskbar jump list: a "Recently played" category plus transport
//! tasks. Each entry starts `glyph --dock-action N`, which the single-instance
//! handler (`crate::instance`) forwards to the running app.
//!
//! GPUI's own jump list only knows a fixed "Tasks" section, so the list is
//! written here directly through `ICustomDestinationList`.

use windows::core::{Interface, HSTRING};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::Common::IObjectCollection;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink};

/// `PKEY_Title`: the text Windows shows for a jump list entry.
const PKEY_TITLE: PROPERTYKEY = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0xf29f85e0_4ff9_1068_ab91_08002b27b3d9), pid: 2 };

/// One entry: label shown in the menu and the dock-action index it triggers.
pub struct Entry {
    pub label: String,
    pub action: usize,
}

fn link(label: &str, action: usize) -> windows::core::Result<IShellLinkW> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let exe = std::env::current_exe().unwrap_or_default();
        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        link.SetArguments(&HSTRING::from(format!("--dock-action {action}")))?;
        link.SetDescription(&HSTRING::from(label))?;
        link.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;
        let store: IPropertyStore = link.cast()?;
        store.SetValue(&PKEY_TITLE, &PROPVARIANT::from(label))?;
        store.Commit()?;
        Ok(link)
    }
}

/// Replaces the jump list. Runs on its own thread (COM apartment of its own).
pub fn update(category: String, recent: Vec<Entry>, tasks: Vec<Entry>) {
    std::thread::spawn(move || {
        let r: windows::core::Result<()> = (|| unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let list: ICustomDestinationList = CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
            let mut slots = 0u32;
            let _removed: windows::Win32::UI::Shell::Common::IObjectArray = list.BeginList(&mut slots)?;
            if !recent.is_empty() {
                let items: IObjectCollection = CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
                for e in &recent {
                    items.AddObject(&link(&e.label, e.action)?)?;
                }
                list.AppendCategory(&HSTRING::from(category.as_str()), &items)?;
            }
            let t: IObjectCollection = CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
            for e in &tasks {
                t.AddObject(&link(&e.label, e.action)?)?;
            }
            list.AddUserTasks(&t)?;
            list.CommitList()?;
            Ok(())
        })();
        if let Err(e) = r {
            tracing::warn!("jump list: {e}");
        }
    });
}
