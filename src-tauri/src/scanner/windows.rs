use super::Binding;
use std::{collections::HashSet, mem::size_of, net::Ipv4Addr, ptr, slice};
use windows_sys::Win32::{
    Foundation::INVALID_HANDLE_VALUE,
    NetworkManagement::{IpHelper::*, Ndis::IfOperStatusUp},
    Networking::WinSock::{
        GetNameInfoW, IpDadStateDeprecated, IpDadStatePreferred, NlnsReachable, NlnsStale,
        WSACleanup, WSAStartup, AF_INET, NI_MAXHOST, NI_NAMEREQD, SOCKADDR_IN, WSADATA,
    },
};

pub(super) fn bindings(adapter: &str, index: u32) -> Result<Vec<Binding>, String> {
    let nic = crate::netcfg::read_interface(adapter, Some(index))?;
    if nic.InterfaceIndex != index || nic.OperStatus != IfOperStatusUp {
        return Err("网卡已断开或发生变化，请重新选择".into());
    }
    let mut table = ptr::null_mut();
    // Windows owns this aligned, variable-length table. Copy the address/prefix pairs before freeing it.
    unsafe {
        let rc = GetUnicastIpAddressTable(AF_INET, &mut table);
        if rc != 0 || table.is_null() {
            return Err(format!("无法读取 IPv4 网段（Windows 错误码 {rc}）"));
        }
        let rows = slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        let result = rows.iter()
            .filter(|r| r.InterfaceIndex == index && r.OnLinkPrefixLength <= 32
                && (r.DadState == IpDadStatePreferred || r.DadState == IpDadStateDeprecated))
            .map(|r| Binding {
                ip: Ipv4Addr::from(r.Address.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes()),
                prefix: r.OnLinkPrefixLength,
                ethernet: matches!(nic.Type, IF_TYPE_ETHERNET_CSMACD | IF_TYPE_IEEE80211),
            }).collect();
        FreeMibTable(table.cast());
        Ok(result)
    }
}

// Cache entries only affect scheduling. Every online result still requires a probe.
pub(super) fn neighbors(index: u32) -> HashSet<Ipv4Addr> {
    let mut table = ptr::null_mut();
    unsafe {
        if GetIpNetTable2(AF_INET, &mut table) != 0 || table.is_null() {
            return HashSet::new();
        }
        let rows = slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        let ips = rows.iter()
            .filter(|r| r.InterfaceIndex == index && r.PhysicalAddressLength != 0
                && (r.State == NlnsReachable || r.State == NlnsStale))
            .map(|r| Ipv4Addr::from(r.Address.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes()))
            .collect();
        FreeMibTable(table.cast());
        ips
    }
}

pub(super) fn probe(source: Binding, ip: Ipv4Addr) -> (bool, Option<String>) {
    let src = u32::from_ne_bytes(source.ip.octets());
    let dst = u32::from_ne_bytes(ip.octets());
    if source.ethernet {
        // SendARP requires room for at least two ULONGs even for a six-byte MAC.
        let mut storage = [0u32; 2];
        let mut len = 8;
        let rc = unsafe { SendARP(dst, src, storage.as_mut_ptr().cast(), &mut len) };
        if rc != 0 || len != 6 { return (false, None); }
        let [a,b,c,d] = storage[0].to_ne_bytes();
        let [e,f,_,_] = storage[1].to_ne_bytes();
        return (true, Some(format!("{a:02X}-{b:02X}-{c:02X}-{d:02X}-{e:02X}-{f:02X}")));
    }
    // Point-to-point and tunnel adapters may not have Ethernet neighbours.
    unsafe {
        let handle = IcmpCreateFile();
        if handle == INVALID_HANDLE_VALUE { return (false, None); }
        let mut reply = [0u64; 16];
        let payload = [0u8; 8];
        let count = IcmpSendEcho2Ex(handle, ptr::null_mut(), None, ptr::null(), src, dst,
            payload.as_ptr().cast(), 8, ptr::null(), reply.as_mut_ptr().cast(), 128, 500);
        IcmpCloseHandle(handle);
        let header = &*reply.as_ptr().cast::<ICMP_ECHO_REPLY>();
        (count != 0 && header.Status == 0 && header.Address == dst, None)
    }
}

pub(super) fn hostname(ip: Ipv4Addr) -> Option<String> {
    let mut addr = SOCKADDR_IN { sin_family: AF_INET, ..Default::default() };
    addr.sin_addr.S_un.S_addr = u32::from_ne_bytes(ip.octets());
    let mut buffer = [0u16; NI_MAXHOST as usize];
    let rc = unsafe {
        if WSAStartup(0x0202, &mut WSADATA::default()) != 0 {
            return None;
        }
        let rc = GetNameInfoW(
            ptr::from_ref(&addr).cast(), size_of::<SOCKADDR_IN>() as i32,
            buffer.as_mut_ptr(), NI_MAXHOST, ptr::null_mut(), 0, NI_NAMEREQD as i32,
        );
        WSACleanup();
        rc
    };
    if rc != 0 { return None; }
    let end = buffer.iter().position(|&unit| unit == 0)?;
    let name = String::from_utf16_lossy(&buffer[..end]);
    let name = name.trim_end_matches('.');
    (!name.is_empty()).then(|| name.to_owned())
}
