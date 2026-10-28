%global __provides_exclude_from ^(%{_bindir}/ak820-(gtk|ctl))
%global _missing_build_ids_terminate_build 0
%global _build_id_links none

Name:           ak820-gtk
Version:        0.1.0
Release:        1.beta%{?dist}
Summary:        GTK3 control app for the AJAZZ / Epomaker AK820 Pro TFT screen

License:        MIT
URL:            https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo rust
BuildRequires:  gcc gcc-c++ make pkgconf-pkg-config
BuildRequires:  gtk3-devel glib2-devel
BuildRequires:  libpng-devel libjpeg-turbo-devel libwebp-devel

Requires:       gtk3 glib2 gdk-pixbuf2

%description
GTK3 control app for the AJAZZ / Epomaker AK820 Pro keyboard screen. Reads device
info, syncs the keyboard clock, uploads images/GIFs with a live preview, manages
upload history and settings. Talks to the keyboard directly over hidraw; the
included udev rule grants non-root access.

%prep
%setup -q

%build
cargo build --release -p ak820-gtk -p ak820-cli

%install
install -Dm755 target/release/ak820-gtk   %{buildroot}%{_bindir}/ak820-gtk
install -Dm755 target/release/ak820ctl   %{buildroot}%{_bindir}/ak820ctl

install -Dm644 udev/70-ak820.rules %{buildroot}/usr/lib/udev/rules.d/70-ak820.rules

install -d %{buildroot}%{_unitdir}
install -m644 systemd/ak820-clock.service %{buildroot}%{_unitdir}/ak820-clock.service
install -m644 systemd/ak820-clock.timer %{buildroot}%{_unitdir}/ak820-clock.timer
install -m644 systemd/ak820-connect.service %{buildroot}%{_unitdir}/ak820-connect.service
install -m644 systemd/ak820-resume.service %{buildroot}%{_unitdir}/ak820-resume.service

install -Dm644 packaging/linux/ak820-gtk.desktop %{buildroot}%{_datadir}/applications/ak820-gtk.desktop
install -Dm644 packaging/linux/ak820-gtk.png %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/ak820-gtk.png

install -Dm644 LICENSE %{buildroot}%{_datadir}/licenses/ak820-gtk/LICENSE
install -Dm644 NOTICE %{buildroot}%{_datadir}/doc/ak820-gtk/NOTICE
install -Dm644 PROTOCOL.md %{buildroot}%{_datadir}/doc/ak820-gtk/PROTOCOL.md
install -Dm644 README.md %{buildroot}%{_datadir}/doc/ak820-gtk/README.md
install -Dm644 docs/UPSTREAM.md %{buildroot}%{_datadir}/doc/ak820-gtk/UPSTREAM.md

%check
cargo test --workspace

%clean
rm -rf %{buildroot}

%files
%{_bindir}/ak820-gtk
%{_bindir}/ak820ctl
/usr/lib/udev/rules.d/70-ak820.rules
%{_unitdir}/ak820-clock.service
%{_unitdir}/ak820-clock.timer
%{_unitdir}/ak820-connect.service
%{_unitdir}/ak820-resume.service
%{_datadir}/applications/ak820-gtk.desktop
%{_datadir}/icons/hicolor/256x256/apps/ak820-gtk.png
%{_datadir}/licenses/ak820-gtk/LICENSE
%{_datadir}/doc/ak820-gtk/NOTICE
%{_datadir}/doc/ak820-gtk/PROTOCOL.md
%{_datadir}/doc/ak820-gtk/README.md
%{_datadir}/doc/ak820-gtk/UPSTREAM.md

%changelog
* Wed Oct 28 2026 Emin G. <emin.by@outlook.com> - 0.1.0-beta-1
- Initial GTK3 release
