%define debug_package %{nil}
BuildRequires: systemd-rpm-macros


Name:       stackable-agent
Version:    0.1.0
Release:    1%{?dist}
Summary:    Binarius package

Group:      System Environment/Base
License:    GPLv3+
Source0:    stackable-agent-0.1.0.tar.gz

%description
Testing package.

%prep
%setup -q #unpack tarball

%build

%install
cp -rfa * %{buildroot}

%post
%systemd_post stackable-agent.service
    /usr/bin/systemctl daemon-reload


%files
/opt/stackable-agent/agent
/etc/stackable-agent/agent.conf
/etc/systemd/system/stackable-agent.service
%dir /var/lib/stackable/data
%dir /var/lib/stackable/config
%dir /var/lib/stackable/packages
