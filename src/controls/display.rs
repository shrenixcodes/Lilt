//! Screen brightness through WMI (`root\wmi`, `WmiMonitorBrightness`), which covers
//! built-in laptop and tablet screens. External monitors usually do not offer it; then
//! Lilt says so and links to Windows' display settings.
//!
//! WMI also needs the process to allow impersonation, which `app::run` sets up once with
//! `CoInitializeSecurity`; without it the brightness classes refuse access.

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoSetProxyBlanket, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_INFINITE,
    WbemLocator,
};
use windows::core::{BSTR, Interface, PCWSTR, w};

fn services() -> Option<IWbemServices> {
    unsafe {
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).ok()?;
        let services = locator
            .ConnectServer(&BSTR::from(r"ROOT\WMI"), &BSTR::new(), &BSTR::new(), &BSTR::new(), 0, &BSTR::new(), None)
            .ok()?;
        impersonate(&services);
        Some(services)
    }
}

/// WMI's brightness classes refuse callers that do not let it impersonate them.
fn impersonate(proxy: &impl Interface) {
    let Ok(proxy) = proxy.cast::<windows::core::IUnknown>() else { return };
    // 10: Windows authentication (RPC_C_AUTHN_WINNT); 0: no authorization service.
    let _ = unsafe {
        CoSetProxyBlanket(
            &proxy,
            10,
            0,
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )
    };
}

/// The first active object returned by a WQL query.
fn first(services: &IWbemServices, query: &str) -> Option<IWbemClassObject> {
    unsafe {
        let objects = services
            .ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(query),
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )
            .ok()?;
        impersonate(&objects);
        let mut found = [None];
        let mut returned = 0u32;
        objects.Next(WBEM_INFINITE, &mut found, &mut returned).ok().ok()?;
        found[0].take()
    }
}

/// The built-in screen's brightness, 0 to 100, or `None` if it cannot be read.
pub fn brightness() -> Option<u32> {
    let services = services()?;
    let object = first(&services, "SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active = TRUE")?;
    let mut value = VARIANT::default();
    unsafe { object.Get(w!("CurrentBrightness"), 0, &mut value, None, None) }.ok()?;
    u32::try_from(&value).ok().map(|v| v.min(100))
}

pub fn set_brightness(level: u32) -> bool {
    let Some(services) = services() else { return false };
    let Some(target) = first(&services, "SELECT * FROM WmiMonitorBrightnessMethods WHERE Active = TRUE") else {
        return false;
    };
    unsafe {
        let mut path = VARIANT::default();
        if target.Get(w!("__PATH"), 0, &mut path, None, None).is_err() {
            return false;
        }
        let Ok(path) = BSTR::try_from(&path) else { return false };
        let mut class = None;
        if services
            .GetObject(&BSTR::from("WmiMonitorBrightnessMethods"), Default::default(), None, Some(&mut class), None)
            .is_err()
        {
            return false;
        }
        let Some(class) = class else { return false };
        let mut signature = None;
        if class.GetMethod(w!("WmiSetBrightness"), 0, &mut signature, std::ptr::null_mut()).is_err() {
            return false;
        }
        let Some(params) = signature.and_then(|s| s.SpawnInstance(0).ok()) else { return false };
        let timeout = VARIANT::from(0i32);
        let brightness = VARIANT::from(level.min(100) as u8);
        if params.Put(w!("Timeout"), 0, &timeout, 0).is_err()
            || params.Put(w!("Brightness"), 0, &brightness, 0).is_err()
        {
            return false;
        }
        services
            .ExecMethod(&path, &BSTR::from("WmiSetBrightness"), Default::default(), None, &params, None, None)
            .is_ok()
    }
}
