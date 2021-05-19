# Changelog

## 0.1.1 - 2021-05-20

### Fixed
- Pod state synchronized with systemd service state ([#164]).

[#164]: https://github.com/stackabletech/agent/pull/164

## 0.1.0 - 2021-05-17

### Added
- Apache license v2.0 set ([#23]).
- Krustlet based agent implementation created ([#1], [#18], [#26], [#35], [#40]).
- Functionality to stop and restart processes added ([#25]).
- Agent restart without impacting running services enabled ([#63]).
- Rendering of template variables to environment variables added ([#30]).
- Setting of pod condition "ready" for state "running" added ([#32]).
- Support for command line parameters added ([#36], [#50], [#72], [#109]).
- Integration with systemd implemented ([#43], [#53], [#100], [#152]).
- Dependabot and security audit enabled ([#56], [#57]).
- Building and publishing of nightly deb and rpm packages added ([#73], [#78], [#94], [#110], [#144]).
- Bootstrapping of certificates and kubeconfig added ([#77]).
- Support for running of services as application users added ([#79]).
- Retrieval of container logs with kubectl logs implemented ([#135]).
- Configuration of terminationGracePeriodSeconds considered in systemd units ([#138]).
- Systemd dependency adapted so that it is compatible with systemd version 241 ([#145]).

[#1]: https://github.com/stackabletech/agent/pull/1
[#18]: https://github.com/stackabletech/agent/pull/18
[#23]: https://github.com/stackabletech/agent/pull/23
[#25]: https://github.com/stackabletech/agent/pull/25
[#26]: https://github.com/stackabletech/agent/pull/26
[#30]: https://github.com/stackabletech/agent/pull/30
[#32]: https://github.com/stackabletech/agent/pull/32
[#35]: https://github.com/stackabletech/agent/pull/35
[#36]: https://github.com/stackabletech/agent/pull/36
[#40]: https://github.com/stackabletech/agent/pull/40
[#43]: https://github.com/stackabletech/agent/pull/43
[#50]: https://github.com/stackabletech/agent/pull/50
[#53]: https://github.com/stackabletech/agent/pull/53
[#56]: https://github.com/stackabletech/agent/pull/56
[#57]: https://github.com/stackabletech/agent/pull/57
[#63]: https://github.com/stackabletech/agent/pull/63
[#72]: https://github.com/stackabletech/agent/pull/72
[#73]: https://github.com/stackabletech/agent/pull/73
[#77]: https://github.com/stackabletech/agent/pull/77
[#78]: https://github.com/stackabletech/agent/pull/78
[#79]: https://github.com/stackabletech/agent/pull/79
[#94]: https://github.com/stackabletech/agent/pull/94
[#100]: https://github.com/stackabletech/agent/pull/100
[#109]: https://github.com/stackabletech/agent/pull/109
[#110]: https://github.com/stackabletech/agent/pull/110
[#135]: https://github.com/stackabletech/agent/pull/135
[#138]: https://github.com/stackabletech/agent/pull/138
[#144]: https://github.com/stackabletech/agent/pull/144
[#145]: https://github.com/stackabletech/agent/pull/145
[#152]: https://github.com/stackabletech/agent/pull/152
