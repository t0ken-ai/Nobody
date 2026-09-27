//! Multicast discovery publishes routing hints, never authority or payload.
//! IPv4 private/link-local interfaces are used in v1; public interfaces do not
//! advertise the service and the listener separately filters incoming addresses.
use super::{bounded_name, platform_name, Peer, Service};
use mdns_sd::{IfKind, IfPredicate, ServiceDaemon, ServiceEvent, ServiceInfo};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
pub const SERVICE: &str = "_translateme._tcp.local.";
/// The loopback exception is for local diagnostics, not LAN advertisement.
pub fn local_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => a.is_private() || a.is_link_local() || a.is_loopback(),
        IpAddr::V6(_) => false,
    }
}
/// Register one service and drive discovery independently of the UI. Closing the
/// app's main window leaves this worker running; the lifecycle token stops it.
pub fn start(
    service: Service,
    port: u16,
    id: &str,
    stop: CancellationToken,
) -> Result<(ServiceDaemon, String), String> {
    let daemon = ServiceDaemon::new().map_err(|e| format!("局域网发现启动失败：{e}"))?;
    let setup = (|| {
        daemon
            .disable_interface(IfKind::All)
            .map_err(|e| e.to_string())?;
        daemon
            .enable_interface(IfKind::Predicate(IfPredicate::new(|i| {
                local_address(i.ip()) && !i.ip().is_loopback()
            })))
            .map_err(|e| e.to_string())?;
        let name = service.inner.lock().unwrap().settings.name.clone();
        let platform = platform_name();
        let props = [
            ("id", id),
            ("name", name.as_str()),
            ("os", platform.as_str()),
            ("version", "1"),
        ];
        let info = ServiceInfo::new(
            SERVICE,
            &id[..16],
            &format!("tm-{}.local.", &id[..24]),
            "",
            port,
            &props[..],
        )
        .map_err(|e| e.to_string())?
        .enable_addr_auto();
        let fullname = info.get_fullname().to_owned();
        let browser = daemon.browse(SERVICE).map_err(|e| e.to_string())?;
        let monitor = daemon.monitor().map_err(|e| e.to_string())?;
        daemon.register(info).map_err(|e| e.to_string())?;
        Ok::<_, String>((fullname, browser, monitor))
    })();
    let (fullname, browser, monitor) = match setup {
        Ok(v) => v,
        Err(e) => {
            let _ = daemon.shutdown();
            return Err(e);
        }
    };
    let watcher = daemon.clone();
    let own_id = id.to_owned();
    tokio::spawn(async move {
        let mut verification = tokio::time::interval(Duration::from_secs(20));
        loop {
            tokio::select! {
                _=stop.cancelled()=>break,
                _=verification.tick()=>{
                    let names:Vec<_>=service.inner.lock().unwrap().peers.values().filter(|p|p.online).map(|p|p.service_name.clone()).collect();
                    for name in names{let _=watcher.verify(name,Duration::from_secs(5));}
                }
                event=monitor.recv_async()=>{
                    match event{Ok(mdns_sd::DaemonEvent::Error(e))=>{service.inner.lock().unwrap().error=format!("局域网发现：{e}");service.emit("changed");},Err(_)=>{service.network_failed(&stop,"局域网发现服务已结束，请重新连接。".into()).await;break;},_=>{}}
                }
                event=browser.recv_async()=>match event{
                    Ok(ServiceEvent::ServiceResolved(info))=>{
                        let prop=|k|info.get_property_val_str(k).unwrap_or("");
                        let id=prop("id");
                        if id==own_id||id.len()!=64||!id.bytes().all(|b|b.is_ascii_hexdigit())||prop("version")!="1"{continue;}
                        let Ok(name)=bounded_name(prop("name"))else{continue;};
                        let mut addresses:Vec<_>=info.get_addresses_v4().into_iter().filter(|a|local_address((*a).into())&&!a.is_loopback()).map(|a|SocketAddr::new(a.into(),info.get_port())).collect();
                        // mDNS address sets have no stable order. Normalize
                        // before comparing so periodic verification of an
                        // unchanged peer does not wake the UI every 20 seconds.
                        addresses.sort_unstable(); addresses.truncate(8);
                        if addresses.is_empty(){continue;}
                        let platform=match prop("os"){"macOS"=>"macOS","Windows"=>"Windows",_=>"未知系统"}.into();
                        let mut i=service.inner.lock().unwrap();
                        let peer=Peer{id:id.into(),name,platform,addresses,online:true,trusted:false,service_name:info.get_fullname().into()};
                        let changed=(i.peers.len()<256||i.peers.contains_key(id))&&i.peers.get(id)!=Some(&peer);
                        if changed{i.peers.insert(id.into(),peer);}
                        drop(i);if changed{service.emit("changed");}
                    }
                    Ok(ServiceEvent::ServiceRemoved(_,fullname))=>{
                        let mut i=service.inner.lock().unwrap();let mut changed=false;
                        for p in i.peers.values_mut().filter(|p|p.service_name==fullname&&p.online){p.online=false;changed=true;}
                        drop(i);if changed{service.emit("changed");}
                    }
                    Err(_)=>{service.network_failed(&stop,"局域网发现服务已结束，请重新连接。".into()).await;break;},
                    _=>{}
                }
            }
        }
    });
    Ok((daemon, fullname))
}
