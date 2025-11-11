use std::net::IpAddr;

use cursive::{
    view::ViewWrapper,
    views::{EditView, SelectView},
};

use super::{CidrAddressEditView, FormView};
use proxmox_installer_common::{
    options::NetworkOptions,
    setup::NetworkInfo,
    utils::{CidrAddress, Fqdn},
};

pub struct NetworkOptionsView {
    view: FormView,
}

impl NetworkOptionsView {
    pub fn new(options: &NetworkOptions, network_info: &NetworkInfo) -> Self {
        let ifaces = network_info.interfaces.values();
        let ifnames = ifaces
            .clone()
            .map(|iface| (iface.render(), iface.name.clone()));
        let mut ifaces_selection = SelectView::new().popup().with_all(ifnames.clone());

        // sort first to always have stable view
        ifaces_selection.sort();
        let selected = ifaces_selection
            .iter()
            .position(|(_label, iface)| *iface == options.ifname)
            .unwrap_or(ifaces.len() - 1);

        ifaces_selection.set_selection(selected);

        let view = FormView::new()
            .child("Management interface", ifaces_selection)
            .child(
                "Hostname (FQDN)",
                EditView::new().content(options.fqdn.to_string()),
            )
            .child(
                "IP address (CIDR)",
                CidrAddressEditView::new().content(options.address.clone()),
            )
            .child(
                "Gateway address",
                EditView::new().content(options.gateway.to_string()),
            )
            .child(
                "DNS server address",
                EditView::new().content(options.dns_server.to_string()),
            );

        Self { view }
    }

    pub fn get_values(&mut self) -> Result<NetworkOptions, String> {
        let ifname = self
            .view
            .get_value::<SelectView, _>(0)
            .ok_or("failed to retrieve management interface name")?;

        let fqdn = self
            .view
            .get_value::<EditView, _>(1)
            .ok_or("failed to retrieve host FQDN")?
            .parse::<Fqdn>()
            .map_err(|err| format!("hostname does not look valid:\n\n{err}"))?;

        let address = self
            .view
            .get_value::<CidrAddressEditView, _>(2)
            .ok_or("failed to retrieve host address".to_string())
            .and_then(|(ip_addr, mask)| {
                CidrAddress::new(ip_addr, mask).map_err(|err| err.to_string())
            })?;

        let gateway = self
            .view
            .get_value::<EditView, _>(3)
            .ok_or("failed to retrieve gateway address")?
            .parse::<IpAddr>()
            .map_err(|err| err.to_string())?;

        let dns_server = self
            .view
            .get_value::<EditView, _>(4)
            .ok_or("failed to retrieve DNS server address")?
            .parse::<IpAddr>()
            .map_err(|err| err.to_string())?;

        if address.addr().is_ipv4() != gateway.is_ipv4() {
            Err("host and gateway IP address version must not differ".to_owned())
        } else if address.addr().is_ipv4() != dns_server.is_ipv4() {
            Err("host and DNS IP address version must not differ".to_owned())
        } else if fqdn.to_string().ends_with(".invalid") {
            Err("hostname does not look valid".to_owned())
        } else {
            Ok(NetworkOptions {
                ifname,
                fqdn,
                address,
                gateway,
                dns_server,
            })
        }
    }
}

impl ViewWrapper for NetworkOptionsView {
    cursive::wrap_impl!(self.view: FormView);
}
