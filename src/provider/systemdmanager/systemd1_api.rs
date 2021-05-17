//! Binding to the D-Bus interface of systemd
use fmt::Display;
use inflector::cases::kebabcase;
use serde::{de::Visitor, Deserialize, Serialize};
use std::{
    convert::TryFrom,
    fmt::{self, Formatter},
    str::FromStr,
};
use strum::{AsRefStr, EnumString, EnumVariantNames, VariantNames};
use zbus::dbus_proxy;
use zvariant::{derive::Type, OwnedValue, Signature, Type};

/// Type of an entry in a changes list
#[derive(Debug, EnumString, EnumVariantNames, PartialEq)]
#[strum(serialize_all = "kebab-case")]
pub enum ChangeType {
    Symlink,
    Unlink,
}

impl<'de> Deserialize<'de> for ChangeType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct VariantVisitor;

        impl<'de> Visitor<'de> for VariantVisitor {
            type Value = ChangeType;

            fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
                write!(formatter, "Expecting one of {:?}", Self::Value::VARIANTS)
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                FromStr::from_str(v).map_err(|_| E::unknown_variant(v, Self::Value::VARIANTS))
            }
        }

        deserializer.deserialize_str(VariantVisitor)
    }
}

impl Type for ChangeType {
    fn signature() -> Signature<'static> {
        String::signature()
    }
}

/// Entry of a changes list
#[derive(Debug, Type, Deserialize)]
pub struct Change {
    pub change_type: ChangeType,
    pub filename: String,
    pub destination: String,
}

/// Changes list returned by functions which change unit files
type Changes = Vec<Change>;

/// Mode in which a unit will be started
#[derive(Debug, AsRefStr)]
#[allow(dead_code)]
pub enum StartMode {
    /// The unit and its dependencies will be started, possibly
    /// replacing already queued jobs that conflict with it.
    Replace,

    /// The unit and its dependencies will be started, but will fail if
    /// this would change an already queued job.
    Fail,

    /// The unit in question will be started and all units that aren't
    /// dependencies of it will be terminated.
    Isolate,

    /// The unit will be started but all its dependencies will be
    /// ignored.
    ///
    /// It is not recommended to make use of this mode.
    IgnoreDependencies,

    /// The unit will be started but the requirement dependencies will
    /// be ignored.
    ///
    /// It is not recommended to make use of this mode.
    IgnoreRequirements,
}

impl Serialize for StartMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&kebabcase::to_kebab_case(self.as_ref()))
    }
}

impl Type for StartMode {
    fn signature() -> Signature<'static> {
        String::signature()
    }
}

/// Mode in which a unit will be stopped
#[derive(Debug, AsRefStr)]
#[allow(dead_code)]
pub enum StopMode {
    /// The unit and its dependencies will be stopped, possibly
    /// replacing already queued jobs that conflict with it.
    Replace,

    /// The unit and its dependencies will be stopped, but will fail if
    /// this would change an already queued job.
    Fail,

    /// The unit will be stopped but all its dependencies will be
    /// ignored.
    ///
    /// It is not recommended to make use of this mode.
    IgnoreDependencies,

    /// The unit will be stopped but the requirement dependencies will
    /// be ignored.
    ///
    /// It is not recommended to make use of this mode.
    IgnoreRequirements,
}

impl Serialize for StopMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&kebabcase::to_kebab_case(self.as_ref()))
    }
}

impl Type for StopMode {
    fn signature() -> Signature<'static> {
        String::signature()
    }
}

/// The manager object is the central entry point for clients.
///
/// Currently not all methods of the systemd object are exposed.
///
/// # Examples
///
/// Synchronous API:
///
/// ```
/// # use stackable_agent::provider::systemdmanager::systemd1_api::ManagerProxy;
/// let connection = zbus::Connection::new_system().unwrap();
/// let manager = ManagerProxy::new(&connection).unwrap();
/// let unit = manager.load_unit("dbus.service").unwrap();
/// ```
///
/// Asynchronous API:
///
/// ```
/// # use stackable_agent::provider::systemdmanager::systemd1_api::AsyncManagerProxy;
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let connection = zbus::azync::Connection::new_system().await.unwrap();
/// let manager = AsyncManagerProxy::new(&connection).unwrap();
/// let unit = manager.load_unit("dbus.service").await.unwrap();
/// # });
/// ```
#[dbus_proxy(
    default_service = "org.freedesktop.systemd1",
    interface = "org.freedesktop.systemd1.Manager",
    default_path = "/org/freedesktop/systemd1"
)]
trait Manager {
    /// Loads the unit from disk if possible and returns it.
    #[dbus_proxy(object = "Unit")]
    fn load_unit(&self, name: &str);

    /// Enqueues a start job and possibly depending jobs and returns the
    /// newly created job.
    #[dbus_proxy(object = "Job")]
    fn start_unit(&self, name: &str, mode: StartMode);

    /// Enqueues a stop job and returns the newly created job.
    #[dbus_proxy(object = "Job")]
    fn stop_unit(&self, name: &str, mode: StopMode);

    /// Reloads all unit files.
    fn reload(&self) -> zbus::Result<()>;

    /// Enables one or more units in the system.
    ///
    /// Units are enabled by creating symlinks to them in `/etc/` or
    /// `/run/`.
    ///
    /// `files` takes a list of unit files to enable (either just file
    /// names or full absolute paths if the unit files are residing
    /// outside the usual unit search paths). `runtime` controls whether
    /// the unit shall be enabled for runtime only (`true`, `/run/`), or
    /// persistently (`false`, `/etc/`). `force` controls whether
    /// symlinks pointing to other units shall be replaced if necessary.
    ///
    /// This method returns one boolean and an array of the changes
    /// made. The boolean signals whether the unit files contained any
    /// enablement information (i.e. an `[Install]`) section.
    fn enable_unit_files(
        &self,
        files: &[&str],
        runtime: bool,
        force: bool,
    ) -> zbus::Result<(bool, Changes)>;

    /// Disables one or more units in the system.
    ///
    /// All symlinks to them in `/etc/` and `/run/` are removed.
    ///
    /// `runtime` controls whether the unit shall be disabled for
    /// runtime only (`true`, `/run/`), or persistently (`false`,
    /// `/etc/`).
    fn disable_unit_files(&self, files: &[&str], runtime: bool) -> zbus::Result<Changes>;

    /// Links unit files (that are located outside of the usual unit
    /// search paths) into the unit search path.
    ///
    /// `runtime` controls whether the unit shall be linked for runtime
    /// only (`true`, `/run/`), or persistently (`false`, `/etc/`).
    /// `force` controls whether symlinks pointing to other units shall
    /// be replaced if necessary.
    fn link_unit_files(&self, files: &[&str], runtime: bool, force: bool) -> zbus::Result<Changes>;
}

/// ActiveState contains a state value that reflects whether the unit is
/// currently active or not.
#[derive(Debug, EnumString, PartialEq)]
#[strum(serialize_all = "kebab-case")]
pub enum ActiveState {
    /// The unit is active.
    Active,

    /// The unit is active and currently reloading its configuration.
    Reloading,

    /// The unit is inactive and the previous run was successful or no
    /// previous run has taken place yet.
    Inactive,

    /// The unit is inactive and the previous run was not successful
    /// (more information about the reason for this is available on the
    /// unit type specific interfaces).
    Failed,

    /// The unit has previously been inactive but is currently in the
    /// process of entering an active state.
    Activating,

    /// The unit is currently in the process of deactivation.
    Deactivating,
}

impl TryFrom<OwnedValue> for ActiveState {
    type Error = zvariant::Error;

    fn try_from(value: OwnedValue) -> Result<Self, Self::Error> {
        FromStr::from_str(&String::try_from(value)?)
            .map_err(|e: strum::ParseError| Self::Error::Message(e.to_string()))
    }
}

/// Unique ID for a runtime cycle of a unit
pub struct InvocationId(Vec<u8>);

impl TryFrom<OwnedValue> for InvocationId {
    type Error = zvariant::Error;

    fn try_from(value: OwnedValue) -> Result<Self, Self::Error> {
        TryFrom::try_from(value).map(InvocationId)
    }
}

impl Display for InvocationId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

/// A systemd unit object
///
/// A [`UnitProxy`] can be retrieved e.g. by [`ManagerProxy::load_unit`].
///
/// Currently not all methods of the systemd object are exposed.
#[dbus_proxy(
    default_service = "org.freedesktop.systemd1",
    interface = "org.freedesktop.systemd1.Unit"
)]
trait Unit {
    /// The active state (i.e. whether the unit is currently started or
    /// not)
    #[dbus_proxy(property)]
    fn active_state(&self) -> zbus::Result<ActiveState>;

    /// Unique ID for a runtime cycle of a unit
    #[dbus_proxy(property, name = "InvocationID")]
    fn invocation_id(&self) -> zbus::Result<InvocationId>;
}

/// A systemd job object
///
/// The [`JobProxy`] is returned by various functions in [`ManagerProxy`].
///
/// Currently no methods of the systemd object are exposed.
#[dbus_proxy(
    default_service = "org.freedesktop.systemd1",
    interface = "org.freedesktop.systemd1.Job"
)]
trait Job {}

#[cfg(test)]
mod test {
    use super::*;
    use byteorder::LE;
    use zvariant::{EncodingContext, Value};

    #[test]
    fn deserialize_change_type() {
        assert_eq!(ChangeType::Symlink, deserialize(&serialize("symlink")));
    }

    #[test]
    fn serialize_start_mode() {
        assert_eq!(
            serialize("ignore-dependencies"),
            serialize(&StartMode::IgnoreDependencies)
        );
    }

    #[test]
    fn serialize_stop_mode() {
        assert_eq!(
            serialize("ignore-dependencies"),
            serialize(&StopMode::IgnoreDependencies)
        );
    }

    #[test]
    fn try_active_state_from_owned_value() {
        assert_eq!(
            ActiveState::Active,
            ActiveState::try_from(OwnedValue::from(Value::from("active"))).unwrap()
        );
    }

    #[test]
    fn invocation_id_to_string() {
        let invocation_id = InvocationId(vec![
            0xbe, 0x44, 0xae, 0xfc, 0xa3, 0xbf, 0x46, 0xba, 0xb0, 0x4b, 0x37, 0x52, 0x09, 0x5d,
            0xd9, 0x97,
        ]);
        assert_eq!(
            "be44aefca3bf46bab04b3752095dd997",
            invocation_id.to_string()
        );
    }

    fn serialize<T: ?Sized>(value: &T) -> Vec<u8>
    where
        T: Serialize + Type,
    {
        let context = EncodingContext::<LE>::new_dbus(0);
        zvariant::to_bytes(context, value).unwrap()
    }

    fn deserialize<'a, T>(bytes: &'a [u8]) -> T
    where
        T: Deserialize<'a> + Type,
    {
        let context = EncodingContext::<LE>::new_dbus(0);
        zvariant::from_slice(bytes, context).unwrap()
    }
}
