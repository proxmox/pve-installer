use cursive::{
    Cursive, View,
    view::{Nameable, Resizable, ViewWrapper},
    views::{
        Button, Checkbox, Dialog, DummyView, EditView, LinearLayout, NamedView, ResizedView,
        ScrollView, SelectView, TextView,
    },
};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
};

use super::{CidrAddressEditView, FormView};
use proxmox_installer_common::{
    net::MAX_IFNAME_LEN,
    options::{NetworkInterfacePinningOptions, NetworkOptions},
    setup::{Interface, NetworkInfo},
    utils::{CidrAddress, Fqdn},
};

struct NetworkViewOptions {
    selected_mac: String,
    pinning_enabled: bool,
    // For UI purposes, we want to always save the mapping, to save the state
    // between toggling the checkbox
    pinning_options: NetworkInterfacePinningOptions,
}

/// Convenience wrapper when needing to take a (interior-mutable) reference to
/// `NetworkViewOptions`.
type NetworkViewOptionsRef = Arc<Mutex<NetworkViewOptions>>;

/// View for configuring anything related to network setup.
pub struct NetworkOptionsView {
    view: LinearLayout,
    options: NetworkViewOptionsRef,
}

impl NetworkOptionsView {
    const PINNING_OPTIONS_BUTTON_NAME: &str = "network-pinning-options-button";
    const MGMT_IFNAME_SELECTVIEW_NAME: &str = "network-management-ifname-selectview";

    pub fn new(options: &NetworkOptions, network_info: &NetworkInfo) -> Self {
        let mut ifaces = network_info
            .interfaces
            .values()
            .collect::<Vec<&Interface>>();

        // First, sort interfaces by their link state and then name
        ifaces.sort_unstable_by_key(|x| (&x.state, &x.name));

        let selected_mac = network_info
            .interfaces
            .get(&options.ifname)
            .map(|iface| iface.mac.clone())
            .unwrap_or_else(|| {
                ifaces
                    .first()
                    .expect("at least one network interface")
                    .mac
                    .clone()
            });

        let options_ref = Arc::new(Mutex::new(NetworkViewOptions {
            selected_mac,
            pinning_enabled: options.pinning_opts.is_some(),
            pinning_options: options.pinning_opts.clone().unwrap_or_default(),
        }));

        let iface_selection =
            Self::build_mgmt_ifname_selectview(ifaces.clone(), options_ref.clone());

        let form = FormView::<()>::new()
            .child(
                "Management interface",
                iface_selection.with_name(Self::MGMT_IFNAME_SELECTVIEW_NAME),
            )
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

        let pinning_checkbox = LinearLayout::horizontal()
            .child(Checkbox::new().checked().on_change({
                let ifaces = ifaces
                    .iter()
                    .map(|iface| (*iface).clone())
                    .collect::<Vec<Interface>>();
                let options_ref = options_ref.clone();
                move |siv, enable_pinning| {
                    siv.call_on_name(Self::PINNING_OPTIONS_BUTTON_NAME, {
                        let options_ref = options_ref.clone();
                        move |view: &mut Button| {
                            view.set_enabled(enable_pinning);

                            options_ref.lock().expect("unpoisoned lock").pinning_enabled =
                                enable_pinning;
                        }
                    });

                    Self::refresh_ifname_selectview(siv, &ifaces, options_ref.clone());
                }
            }))
            .child(TextView::new(" Pin network interface names").no_wrap())
            .child(DummyView.full_width())
            .child(
                Button::new("Pinning options", {
                    let options_ref = options_ref.clone();
                    let network_info = network_info.clone();
                    move |siv| {
                        let mut view =
                            Self::custom_name_mapping_view(&network_info, options_ref.clone());

                        // Pre-compute the child's layout, since it might depend on the size. Without this,
                        // the view will be empty until focused.
                        // The screen size never changes in our case, so this is completely OK.
                        view.layout(siv.screen_size());

                        siv.add_layer(view);
                    }
                })
                .with_name(Self::PINNING_OPTIONS_BUTTON_NAME),
            );

        let view = LinearLayout::vertical()
            .child(form)
            .child(DummyView.full_width())
            .child(pinning_checkbox);

        Self {
            view,
            options: options_ref,
        }
    }

    pub fn get_values(&mut self) -> Result<NetworkOptions, String> {
        let form = self
            .view
            .get_child(0)
            .and_then(|v| v.downcast_ref::<FormView>())
            .ok_or("failed to retrieve network options form")?;

        let iface = form
            .get_value::<NamedView<SelectView<Interface>>, _>(0)
            .ok_or("failed to retrieve management interface name")?;

        let fqdn = form
            .get_value::<EditView, _>(1)
            .ok_or("failed to retrieve host FQDN")?
            .parse::<Fqdn>()
            .map_err(|err| format!("hostname does not look valid:\n\n{err}"))?;

        let address = form
            .get_value::<CidrAddressEditView, _>(2)
            .ok_or("failed to retrieve host address".to_string())
            .and_then(|(ip_addr, mask)| {
                CidrAddress::new(ip_addr, mask).map_err(|err| err.to_string())
            })?;

        let gateway = form
            .get_value::<EditView, _>(3)
            .ok_or("failed to retrieve gateway address")?
            .parse::<IpAddr>()
            .map_err(|err| err.to_string())?;

        let dns_server = form
            .get_value::<EditView, _>(4)
            .ok_or("failed to retrieve DNS server address")?
            .parse::<IpAddr>()
            .map_err(|err| err.to_string())?;

        let pinning_opts = self
            .options
            .lock()
            .map(|opt| opt.pinning_enabled.then_some(opt.pinning_options.clone()))
            .map_err(|err| err.to_string())?;

        let ifname = if let Some(opts) = &pinning_opts
            && let Some(pinned) = iface.to_pinned(opts)
        {
            pinned.name
        } else {
            iface.name
        };

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
                pinning_opts,
            })
        }
    }

    fn custom_name_mapping_view(
        network_info: &NetworkInfo,
        options_ref: NetworkViewOptionsRef,
    ) -> impl View {
        const DIALOG_NAME: &str = "network-interface-name-pinning-dialog";

        let mut interfaces = network_info
            .interfaces
            .values()
            .collect::<Vec<&Interface>>();

        interfaces.sort_by(|a, b| (&a.state, &a.name).cmp(&(&b.state, &b.name)));

        Dialog::around(InterfacePinningOptionsView::new(
            &interfaces,
            options_ref.clone(),
        ))
        .title("Interface Name Pinning Options")
        .button("Ok", {
            let interfaces = interfaces
                .iter()
                .map(|v| (*v).clone())
                .collect::<Vec<Interface>>();
            move |siv| {
                let options = siv
                    .call_on_name(DIALOG_NAME, |view: &mut Dialog| {
                        view.get_content_mut()
                            .downcast_mut::<InterfacePinningOptionsView>()
                            .map(InterfacePinningOptionsView::get_values)
                    })
                    .flatten();

                let options = match options {
                    Some(Ok(options)) => options,
                    Some(Err(err)) => {
                        siv.add_layer(Dialog::info(err));
                        return;
                    }
                    None => {
                        siv.add_layer(Dialog::info(
                            "Failed to retrieve network interface name pinning options view",
                        ));
                        return;
                    }
                };

                siv.pop_layer();
                options_ref.lock().expect("unpoisoned lock").pinning_options = options;

                Self::refresh_ifname_selectview(siv, &interfaces, options_ref.clone());
            }
        })
        .with_name(DIALOG_NAME)
        .max_size((80, 40))
    }

    fn refresh_ifname_selectview(
        siv: &mut Cursive,
        ifaces: &[Interface],
        options_ref: NetworkViewOptionsRef,
    ) {
        siv.call_on_name(
            Self::MGMT_IFNAME_SELECTVIEW_NAME,
            |view: &mut SelectView<Interface>| {
                *view = Self::build_mgmt_ifname_selectview(ifaces.iter().collect(), options_ref);
            },
        );
    }

    fn build_mgmt_ifname_selectview(
        ifaces: Vec<&Interface>,
        options_ref: NetworkViewOptionsRef,
    ) -> SelectView<Interface> {
        let options = options_ref.lock().expect("unpoisoned lock");

        // Map all interfaces to a list of (human-readable interface name, [Interface]) pairs
        let ifnames = ifaces
            .iter()
            .map(|iface| {
                if options.pinning_enabled
                    && let Some(pinned) = iface.to_pinned(&options.pinning_options)
                {
                    (pinned.render(), pinned.clone())
                } else {
                    (iface.render(), (*iface).clone())
                }
            })
            .collect::<Vec<(String, Interface)>>();

        let mut view = SelectView::new()
            .popup()
            .with_all(ifnames.clone())
            .on_submit({
                let options_ref = options_ref.clone();
                move |_, iface| {
                    options_ref.lock().expect("unpoisoned lock").selected_mac = iface.mac.clone();
                }
            });

        // Finally, (try to) select the current one
        let selected = view
            .iter()
            .position(|(_label, iface)| iface.mac == options.selected_mac)
            .unwrap_or(0); // we sort UP interfaces first, so select the first UP interface
        //
        view.set_selection(selected);

        view
    }
}

impl ViewWrapper for NetworkOptionsView {
    cursive::wrap_impl!(self.view: LinearLayout);
}

struct InterfacePinningOptionsView {
    view: ScrollView<NamedView<FormView<String>>>,
}

impl InterfacePinningOptionsView {
    const FORM_NAME: &str = "network-interface-name-pinning-form";

    fn new(interfaces: &[&Interface], options_ref: NetworkViewOptionsRef) -> Self {
        let options = options_ref.lock().expect("unpoisoned lock");

        // Filter out all non-physical links, as it does not make sense to pin their names
        // in this way.
        // The low-level installer will skip them anyway.
        let interfaces = interfaces.iter().filter(|iface| iface.pinned_id.is_some());

        let mut form = FormView::<String>::new();

        for iface in interfaces {
            let label = format!(
                "{} ({}, {}, {})",
                iface.mac, iface.name, iface.driver, iface.state
            );

            let view = LinearLayout::horizontal()
                .child(DummyView.full_width()) // right align below form elements
                .child(
                    EditView::new()
                        .content(
                            iface
                                .to_pinned(&options.pinning_options)
                                .expect("always pinnable interface")
                                .name,
                        )
                        .max_content_width(MAX_IFNAME_LEN)
                        .fixed_width(MAX_IFNAME_LEN),
                );

            form.add_child_with_data(&label, view, iface.mac.clone());

            if !iface.addresses.is_empty() {
                for chunk in iface.addresses.chunks(2) {
                    let addrs = chunk
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<String>>()
                        .join(", ");

                    form.add_child_with_custom_label(&format!("  {addrs}\n"), DummyView);
                }
            }
        }

        Self {
            view: ScrollView::new(form.with_name(Self::FORM_NAME)),
        }
    }

    fn get_values(&mut self) -> Result<NetworkInterfacePinningOptions, String> {
        let form = self.view.get_inner_mut().get_mut();

        let mut mapping = HashMap::new();

        for i in 0..form.len() {
            let (inner, mac) = match form.get_child_with_data::<LinearLayout>(i) {
                Some(formdata) => formdata,
                None => continue,
            };

            let name = inner
                .get_child(1)
                .and_then(|v| v.downcast_ref::<ResizedView<EditView>>())
                .map(|v| v.get_inner().get_content())
                .ok_or_else(|| format!("failed to retrieve pinning ID for interface {}", mac))?;

            mapping.insert(mac.clone(), (*name).clone());
        }

        let opts = NetworkInterfacePinningOptions { mapping };
        opts.verify().map_err(|err| err.to_string())?;

        Ok(opts)
    }
}

impl ViewWrapper for InterfacePinningOptionsView {
    cursive::wrap_impl!(self.view: ScrollView<NamedView<FormView<String>>>);
}
