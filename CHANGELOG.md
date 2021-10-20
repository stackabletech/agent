# Changelog

## [Unreleased]
### Changed
- Agent now also accepts "application/x-tgz" as content_type when downloading packages ([#337])

[#337]: https://github.com/stackabletech/agent/pull/337

### Added
- Cleanup stage added where systemd units without corresponding pods are
  removed on startup ([#312]).

### Changed
- Changed the version reported by the Stackable Agent in `nodeInfo.kubeletVersion` of the `Node` object in Kubernetes
  from the version of the Krustlet library to the Stackable Agent version ([#315]).
- Restart agent on all crashes ([#318]).
- Agent will now request content type "application/gzip" in package downloads and reject responses with content type
  that is not one of either "application/gzip", "application/tgz" or "application/x-gzip" ([#326])

### Fixed
- Agent deletes directories from failed install attempts ([#326])

[#312]: https://github.com/stackabletech/agent/pull/312
[#315]: https://github.com/stackabletech/agent/pull/315
[#318]: https://github.com/stackabletech/agent/pull/318
[#326]: https://github.com/stackabletech/agent/pull/326

## [0.6.1] - 2021-09-14

### Changed
- Changed the binary location for APT packages from
  `/opt/stackable-agent/stackable-agent` to
  `/opt/stackable/stackable-agent/stackable-agent` ([#304]).

[#304]: https://github.com/stackabletech/agent/pull/304

## [0.6.0] - 2021-09-08

### Added
- Prints self-diagnostic information on startup ([#270]).
- Check added on startup if the configured directories exist and are
  writable by the Stackable agent ([#273]).
- Missing directories are created ([#274]).
- Annotation `featureRestartCount` added to the pods to indicate if the
  restart count is set properly ([#289]).

### Changed
- Lazy validation of repository URLs changed to eager validation
  ([#262]).
- `certificates.k8s.io/v1` used instead of `certificates.k8s.io/v1beta1`
  so that the Stackable Agent is now compatible with Kubernetes v1.22
  but not any longer with versions prior to v1.19 ([#267]).
- Error message improved which is logged if a systemd unit file cannot
  be created ([#276]).
- Handling of service restarts moved from the Stackable agent to
  systemd ([#263]).

### Removed
- Check removed if a service starts up correctly within 10 seconds.
  systemd manages restarts now and the Stackable agent cannot detect if
  a service is in a restart loop ([#263]).

### Fixed
- Systemd services in session mode are restarted after a reboot
  ([#263]).

[#262]: https://github.com/stackabletech/agent/pull/262
[#263]: https://github.com/stackabletech/agent/pull/263
[#267]: https://github.com/stackabletech/agent/pull/267
[#270]: https://github.com/stackabletech/agent/pull/270
[#273]: https://github.com/stackabletech/agent/pull/273
[#274]: https://github.com/stackabletech/agent/pull/274
[#276]: https://github.com/stackabletech/agent/pull/276
[#289]: https://github.com/stackabletech/agent/pull/289

## [0.5.0] - 2021-07-26

### Added
- `hostIP` and `podIP` added to the pod status ([#224]).
- Environment variable `KUBECONFIG` set in systemd services ([#234]).

### Fixed
- Invalid or unreachable repositories are skipped when searching for
  packages ([#229]).
- Access rights of the private key file restricted to the owner
  ([#235]). The permissions are set when the file is created. On
  existing installations the permissions must be set manually, e.g. with
  `chmod 600 /etc/stackable/stackable-agent/secret/agent.key`.

[#224]: https://github.com/stackabletech/agent/pull/224
[#229]: https://github.com/stackabletech/agent/pull/229
[#234]: https://github.com/stackabletech/agent/pull/234
[#235]: https://github.com/stackabletech/agent/pull/235

## [0.4.0] - 2021-06-23

### Added
- Annotation `featureLogs` added to the pods to indicate if logs can be
  retrieved with `kubectl logs` ([#188]).

### Changed
- Restart setting in systemd units removed because the agent already
  monitors the units and restarts them according to the restart policy
  in the pod spec ([#205]).

### Fixed
- Pods with restart policy `Never` handled correctly ([#205]).

[#188]: https://github.com/stackabletech/agent/pull/188
[#205]: https://github.com/stackabletech/agent/pull/205

## [0.3.0] - 2021-05-27

### Added
- Artifacts for merge requests are created ([#169], [#173]).

### Changed
- Structure of the documentation changed so that it can be incorporated
  into the overall Stackable documentation ([#165]).

### Fixed
- Deadlock fixed which occurred when multiple pods were started or
  stopped simultaneously ([#176]).

[#165]: https://github.com/stackabletech/agent/pull/165
[#169]: https://github.com/stackabletech/agent/pull/169
[#173]: https://github.com/stackabletech/agent/pull/173
[#176]: https://github.com/stackabletech/agent/pull/176

## [0.2.0] - 2021-05-20

### Added
- Templating facility added to the `config-directory` parameter
  ([#159]).

### Fixed
- Pod state synchronized with systemd service state ([#164]).

[#159]: https://github.com/stackabletech/agent/pull/159
[#164]: https://github.com/stackabletech/agent/pull/164

## [0.1.0] - 2021-05-17

### Added
- Apache license v2.0 set ([#23]).
- Krustlet based agent implementation created ([#1], [#18], [#26],
  [#35], [#40]).
- Functionality to stop and restart processes added ([#25]).
- Agent restart without impacting running services enabled ([#63]).
- Rendering of template variables to environment variables added
  ([#30]).
- Setting of pod condition "ready" for state "running" added ([#32]).
- Support for command line parameters added ([#36], [#50], [#72],
  [#109]).
- Integration with systemd implemented ([#43], [#53], [#100], [#152]).
- Dependabot and security audit enabled ([#56], [#57]).
- Building and publishing of nightly deb and rpm packages added ([#73],
  [#78], [#94], [#110], [#144]).
- Bootstrapping of certificates and kubeconfig added ([#77]).
- Support for running of services as application users added ([#79]).
- Retrieval of container logs with kubectl logs implemented ([#135]).
- Configuration of terminationGracePeriodSeconds considered in systemd
  units ([#138]).
- Systemd dependency adapted so that it is compatible with systemd
  version 241 ([#145]).

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
