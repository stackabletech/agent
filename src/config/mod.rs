use anyhow::anyhow;
use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use log::{debug, error, info, trace};
use nix::ifaddrs;
use nix::net::if_::InterfaceFlags;
use nix::sys::socket::SockAddr;
use stackable_config::{ConfigOption, Configurable, Configuration};
use thiserror::Error;

use crate::config::AgentConfigError::{ArgumentParseError, WrongArgumentCount};
use crate::fsext::{is_valid_file_path, normalize_path};

#[derive(Error, Debug)]
pub enum AgentConfigError {
    #[error("Wrong number of arguments found for config option {}!", .option.name)]
    WrongArgumentCount { option: ConfigOption },
    #[error("Unable to parse value for parameter [{}]!", .name)]
    ArgumentParseError { name: String },
}

#[derive(Clone)]
pub struct AgentConfig {
    pub hostname: String,
    pub parcel_directory: PathBuf,
    pub config_directory: PathBuf,
    pub log_directory: PathBuf,
    pub bootstrap_file: PathBuf,
    pub data_directory: PathBuf,
    pub server_ip_address: IpAddr,
    pub server_port: u16,
    pub server_cert_file: PathBuf,
    pub server_key_file: PathBuf,
    pub tags: HashMap<String, String>,
    pub session: bool,
    pub pod_cidr: String,
}

impl AgentConfig {
    pub const HOSTNAME: ConfigOption = ConfigOption {
        name: "hostname",
        default: None,
        required: false,
        takes_argument: true,
        help:
            "The hostname to register the node under in Kubernetes - defaults to system hostname.",
        documentation: include_str!("config_documentation/hostname.adoc"),
        list: false,
    };

    pub const DATA_DIR: ConfigOption = ConfigOption {
        name: "data-directory",
        default: Some("/var/lib/stackable/agent"),
        required: false,
        takes_argument: true,
        help: "The directory where the stackable agent should keep its working data.",
        documentation: include_str!("config_documentation/data_directory.adoc"),
        list: false,
    };

    pub const BOOTSTRAP_FILE: ConfigOption = ConfigOption {
        name: "bootstrap-file",
        default: Some("/etc/stackable/stackable-agent/bootstrap-kubelet.conf"),
        required: false,
        takes_argument: true,
        help: "The bootstrap file to use in case Kubernetes bootstraping is used to add the agent.",
        documentation: include_str!("config_documentation/bootstrap_file.adoc"),
        list: false,
    };

    pub const SERVER_IP_ADDRESS: ConfigOption = ConfigOption {
        name: "server-bind-ip",
        default: None,
        required: false,
        takes_argument: true,
        help: "The local IP to register as the node's ip with the apiserver. Will be automatically set to the first address of the first non-loopback interface if not specified.",
        documentation: include_str!("config_documentation/server_ip_address.adoc"),
        list: false,
    };

    pub const SERVER_CERT_FILE: ConfigOption = ConfigOption {
        name: "server-cert-file",
        default: Some("/etc/stackable/stackable-agent/secret/agent.crt"),
        required: false,
        takes_argument: true,
        help: "The certificate file for the local webserver which the Krustlet starts.",
        documentation: include_str!("config_documentation/server_cert_file.adoc"),
        list: false,
    };

    pub const SERVER_KEY_FILE: ConfigOption = ConfigOption {
        name: "server-key-file",
        default: Some("/etc/stackable/stackable-agent/secret/agent.key"),
        required: false,
        takes_argument: true,
        help:
            "Private key file (in PKCS8 format) to use for the local webserver the Krustlet starts.",
        documentation: include_str!("config_documentation/server_key_file.adoc"),
        list: false,
    };

    pub const SERVER_PORT: ConfigOption = ConfigOption {
        name: "server-port",
        default: Some("3000"),
        required: false,
        takes_argument: true,
        help: "Port to listen on for callbacks.",
        documentation: include_str!("config_documentation/server_port.adoc"),
        list: false,
    };

    pub const PACKAGE_DIR: ConfigOption = ConfigOption {
        name: "package-directory",
        default: Some("/opt/stackable/packages"),
        required: false,
        takes_argument: true,
        help: "The base directory under which installed packages will be stored.",
        documentation: include_str!("config_documentation/package_directory.adoc"),
        list: false,
    };

    pub const CONFIG_DIR: ConfigOption = ConfigOption {
        name: "config-directory",
        default: Some("/etc/stackable/serviceconfig"),
        required: false,
        takes_argument: true,
        help: "The base directory under which configuration will be generated for all executed services.",
        documentation: include_str!("config_documentation/config_directory.adoc"),        
        list: false,
    };

    pub const LOG_DIR: ConfigOption = ConfigOption {
        name: "log-directory",
        default: Some("/var/log/stackable/servicelogs"),
        required: false,
        takes_argument: true,
        help: "The base directory under which log files will be placed for all services.",
        documentation: include_str!("config_documentation/log_directory.adoc"),
        list: false,
    };

    pub const NO_CONFIG: ConfigOption = ConfigOption {
        name: "no-config",
        default: None,
        required: false,
        takes_argument: false,
        help: "If this option is specified, any file referenced in AGENT_CONF environment variable will be ignored.",
        documentation: include_str!("config_documentation/no_config.adoc"),
        list: false,
    };

    pub const TAG: ConfigOption = ConfigOption {
        name: "tag",
        default: None,
        required: false,
        takes_argument: true,
        help: "A \"key=value\" pair that should be assigned to this agent as tag. This can be specified multiple times to assign additional tags.",
        documentation: include_str!("config_documentation/tags.adoc"),
        list: true
    };

    pub const SESSION_SYSTEMD: ConfigOption = ConfigOption {
        name: "session",
        default: None,
        required: false,
        takes_argument: false,
        help: "When specified causes the agent to run services in the session instance of systemd, not the system wide systemd.",
        documentation: include_str!("config_documentation/session.adoc"),
        list: false
    };

    pub const POD_CIDR: ConfigOption = ConfigOption {
        name: "pod-cidr",
        default: Some(""),
        required: false,
        takes_argument: true,
        help: "An IP range in CIDR notation which designates the range that pods assigned to this node should have their ip addresses in.",
        documentation: include_str!("config_documentation/pod_cidr.adoc"),
        list: false
    };

    /// Returns the directory in which the `server_cert_file` is
    /// located.
    ///
    /// If `server_cert_file` contains only a file name then
    /// `Path::new("")` is returned.
    ///
    /// # Panics
    ///
    /// Panics if `server_cert_file` does not contain a file name. An
    /// [`AgentConfig`] which was created by
    /// [`Configurable::parse_values`] always contains a valid file
    /// name.
    pub fn server_cert_file_dir(&self) -> &Path {
        self.server_cert_file
            .parent()
            .expect("server_cert_file should contain a file")
    }

    /// Returns the directory in which the `server_key_file` is located.
    ///
    /// If `server_key_file` contains only a file name then
    /// `Path::new("")` is returned.
    ///
    /// # Panics
    ///
    /// Panics if `server_key_file` does not contain a file name. An
    /// [`AgentConfig`] which was created by
    /// [`Configurable::parse_values`] always contains a valid file
    /// name.
    pub fn server_key_file_dir(&self) -> &Path {
        self.server_key_file
            .parent()
            .expect("server_key_file should contain a file")
    }

    fn get_options() -> HashSet<ConfigOption> {
        [
            AgentConfig::HOSTNAME,
            AgentConfig::DATA_DIR,
            AgentConfig::SERVER_IP_ADDRESS,
            AgentConfig::SERVER_CERT_FILE,
            AgentConfig::SERVER_KEY_FILE,
            AgentConfig::SERVER_PORT,
            AgentConfig::PACKAGE_DIR,
            AgentConfig::CONFIG_DIR,
            AgentConfig::LOG_DIR,
            AgentConfig::NO_CONFIG,
            AgentConfig::TAG,
            AgentConfig::BOOTSTRAP_FILE,
            AgentConfig::SESSION_SYSTEMD,
            AgentConfig::POD_CIDR,
        ]
        .iter()
        .cloned()
        .collect()
    }

    /// Helper method to ensure that for config options which only allow one value only one value
    /// was specified.
    /// In theory this should be unnecessary, as clap already enforces this check, but we still get
    /// a vec, so in theory could have too many values in there - or none (in which case the default
    /// should have been inserted by clap).
    ///
    /// If we get an incorrect number of arguments, a WrongArgumentCount error is returned which will
    /// cause config parsing to panic.
    fn get_exactly_one_string(
        parsed_values: &HashMap<ConfigOption, Option<Vec<String>>>,
        option: &ConfigOption,
    ) -> Result<String, AgentConfigError> {
        debug!(
            "Trying to obtain exactly one value for ConfigOption {}",
            option.name
        );
        trace!(
            "Parsed values for {} from commandline: {:?}",
            option.name,
            parsed_values.get(option)
        );
        if let Some(Some(list_value)) = parsed_values.get(option) {
            if list_value.len() == 1 {
                // We've checked that the list has exactly one value at this point, so no errors should
                // occur after this point - but you never know
                return Ok(list_value[0].to_string());
            } else {
                error!("Got additional, unexpected values for parameter!");
            }
        }
        Err(WrongArgumentCount {
            option: option.clone(),
        })
    }

    /// Helper method to retrieve a path from the config and convert this to a PathBuf directly.
    /// This method assumes that a default value has been specified for this option and panics if
    /// no value can be retrieved (should only happen if assigning the default value fails or
    /// one was not specified)
    ///
    /// # Panics
    /// This function panics if the parsed_values object does not contain a value for the key.
    /// This is due to the fact that we expect a default value to be defined for these parameters,
    /// so if we do not get a value that default value has not been defined or something else went
    /// badly wrong.
    fn get_with_default<T: FromStr>(
        parsed_values: &HashMap<ConfigOption, Option<Vec<String>>>,
        option: &ConfigOption,
        error_list: &mut Vec<AgentConfigError>,
    ) -> Result<T, anyhow::Error> {
        T::from_str(
            &AgentConfig::get_exactly_one_string(parsed_values, option).unwrap_or_else(|_| {
                panic!(
                    "No value present for parameter {} even though it should have a default value!",
                    option.name
                )
            }),
        )
        .map_err(|_| {
            let error = ArgumentParseError {
                name: option.name.to_string(),
            };
            error_list.push(error);
            anyhow!("Error for parameter: {}", option.name)
        })
    }

    /// This tries to find the first non loopback interface with an ip address assigned.
    /// This should usually be the default interface.
    fn get_default_ipaddress() -> Option<IpAddr> {
        match ifaddrs::getifaddrs() {
            Ok(ifaddr_iter) => {
                let maybe_first_ifaddr = ifaddr_iter
                    .filter(|ifaddr| {
                        ifaddr.flags.contains(InterfaceFlags::IFF_UP)
                            && !ifaddr.flags.contains(InterfaceFlags::IFF_LOOPBACK)
                    })
                    .find_map(|ifaddr| {
                        if let Some(SockAddr::Inet(inet_addr)) = ifaddr.address {
                            Some((ifaddr.interface_name, inet_addr.to_std().ip()))
                        } else {
                            None
                        }
                    });

                if let Some((interface_name, inet_addr)) = maybe_first_ifaddr {
                    debug!(
                        "Found interface {} with the ip address {}.",
                        interface_name, inet_addr
                    );
                    Some(inet_addr)
                } else {
                    error!("Error while finding the default interface - delegating ip retrieval to Kubelet.");
                    None
                }
            }
            Err(err) => {
                error!("Error while retrieving the interface addresses: {}", err);
                None
            }
        }
    }

    fn default_hostname() -> anyhow::Result<String> {
        hostname::get()?
            .into_string()
            .map_err(|_| anyhow::anyhow!("invalid utf-8 hostname string"))
    }

    pub fn get_documentation() -> String {
        let mut doc_string = String::new();
        for option in AgentConfig::get_options() {
            doc_string.push_str(&format!("\n\n\n=== {}\n\n", option.name));
            doc_string.push_str(&format!(
                "*Default value*: `{}`\n\n",
                option.default.unwrap_or("No default value")
            ));
            doc_string.push_str(&format!("*Required*: {}\n\n", option.required));
            doc_string.push_str(&format!("*Multiple values:* {}\n\n\n", option.list));

            // We have not yet specified a documentation string for all options, as an interim
            // solution we use the help string for the docs, if no proper doc has been written yet.
            if option.documentation.is_empty() {
                doc_string.push_str(option.help);
            } else {
                doc_string.push_str(option.documentation);
            }
        }
        doc_string
    }
}

impl Configurable for AgentConfig {
    fn get_config_description() -> Configuration {
        Configuration {
            name: "Stackable Agent",
            version: env!("CARGO_PKG_VERSION"),
            about: env!("CARGO_PKG_DESCRIPTION"),
            options: AgentConfig::get_options(),
        }
    }

    fn parse_values(
        parsed_values: HashMap<ConfigOption, Option<Vec<String>>, RandomState>,
    ) -> Result<Self, anyhow::Error> {
        // Parse hostname or lookup local hostname
        let final_hostname =
            AgentConfig::get_exactly_one_string(&parsed_values, &AgentConfig::HOSTNAME)
                .unwrap_or_else(|_| {
                    AgentConfig::default_hostname()
                        .unwrap_or_else(|_| panic!("Unable to get hostname!"))
                });

        // Parse IP Address or lookup default
        let final_ip = if let Ok(ip) =
            AgentConfig::get_exactly_one_string(&parsed_values, &AgentConfig::SERVER_IP_ADDRESS)
        {
            IpAddr::from_str(&ip)
                .unwrap_or_else(|_| panic!("Couldn't parse {} as a valid ip address!", ip))
        } else {
            AgentConfig::get_default_ipaddress()
                .expect("Error getting default ip address, please specify it explicitly!")
        };
        info!("Selected {} as local address to listen on.", final_ip);

        let mut error_list = vec![];

        // Parse directory/file parameters
        // PathBuf::from_str returns an infallible as Error, so cannot fail, hence unwrap is save
        // to use for PathBufs here

        // Parse data directory from values, add any error that occured to the list of errors
        let final_data_dir = AgentConfig::get_with_default(
            &parsed_values,
            &AgentConfig::DATA_DIR,
            error_list.as_mut(),
        )
        .map(|path: PathBuf| normalize_path(&path));

        // Parse bootstrap file from values
        let final_bootstrap_file = AgentConfig::get_with_default(
            &parsed_values,
            &AgentConfig::BOOTSTRAP_FILE,
            error_list.as_mut(),
        )
        .map(|path: PathBuf| normalize_path(&path));

        // Parse log directory
        let final_log_dir = AgentConfig::get_with_default(
            &parsed_values,
            &AgentConfig::LOG_DIR,
            error_list.as_mut(),
        )
        .map(|path: PathBuf| normalize_path(&path));

        // Parse config directory
        let final_config_dir = AgentConfig::get_with_default(
            &parsed_values,
            &AgentConfig::CONFIG_DIR,
            error_list.as_mut(),
        )
        .map(|path: PathBuf| normalize_path(&path));

        // Parse parcel directory
        let final_package_dir = AgentConfig::get_with_default(
            &parsed_values,
            &AgentConfig::PACKAGE_DIR,
            error_list.as_mut(),
        )
        .map(|path: PathBuf| normalize_path(&path));

        // Parse pod cidr
        let final_pod_cidr: Result<String, anyhow::Error> = AgentConfig::get_with_default(
            &parsed_values,
            &AgentConfig::POD_CIDR,
            error_list.as_mut(),
        );

        // Parse cert file
        let final_server_cert_file = AgentConfig::get_with_default(
            &parsed_values,
            &AgentConfig::SERVER_CERT_FILE,
            error_list.as_mut(),
        )
        .map(|path: PathBuf| normalize_path(&path));

        if let Ok(file) = &final_server_cert_file {
            if !is_valid_file_path(file) {
                let error = ArgumentParseError {
                    name: AgentConfig::SERVER_CERT_FILE.name.to_string(),
                };
                error_list.push(error);
            }
        }

        // Parse key file
        let final_server_key_file = AgentConfig::get_with_default(
            &parsed_values,
            &AgentConfig::SERVER_KEY_FILE,
            error_list.as_mut(),
        )
        .map(|path: PathBuf| normalize_path(&path));

        if let Ok(file) = &final_server_key_file {
            if !is_valid_file_path(file) {
                let error = ArgumentParseError {
                    name: AgentConfig::SERVER_KEY_FILE.name.to_string(),
                };
                error_list.push(error);
            }
        }

        let final_port = AgentConfig::get_with_default(
            &parsed_values,
            &AgentConfig::SERVER_PORT,
            error_list.as_mut(),
        );

        let mut final_tags: HashMap<String, String> = HashMap::new();
        if let Some(Some(tags)) = parsed_values.get(&AgentConfig::TAG) {
            for tag in tags {
                let split: Vec<&str> = tag.split('=').collect();
                if split.len() == 2 {
                    // This might panic, but really shouldn't, as we've checked the size of the array
                    final_tags.insert(split[0].to_string(), split[1].to_string());
                } else {
                    // We want to avoid any "unpredictable" behavior like ignoring a malformed
                    // key=value pair with just a log message -> so we panic if this can't be
                    // parsed
                    error_list.push(ArgumentParseError {
                        name: AgentConfig::TAG.name.to_string(),
                    });
                }
            }
        }

        // The first unwrap defaults to none in case the option is not se

        let final_session = parsed_values
            .get(&AgentConfig::SESSION_SYSTEMD)
            .expect(
                "No value for session parameter found in parsed values, this should not happen!",
            )
            .is_some();

        // Panic if we encountered any errors during parsing of the values
        if !error_list.is_empty() {
            panic!(
                "Error parsing command line parameters:\n{}",
                error_list
                    .into_iter()
                    .map(|thiserror| format!("{:?}\n", thiserror))
                    .collect::<String>()
            );
        }

        // These unwraps are ok to panic, if one of them barfs then something went horribly wrong
        // above, as we should have paniced in a "controlled fashion" from the conditional block
        // right before this
        Ok(AgentConfig {
            hostname: final_hostname,
            parcel_directory: final_package_dir.unwrap(),
            config_directory: final_config_dir.unwrap(),
            data_directory: final_data_dir.unwrap(),
            log_directory: final_log_dir.unwrap(),
            bootstrap_file: final_bootstrap_file.unwrap(),
            server_ip_address: final_ip,
            server_port: final_port.unwrap(),
            server_cert_file: final_server_cert_file.unwrap(),
            server_key_file: final_server_key_file.unwrap(),
            tags: final_tags,
            session: final_session,
            pod_cidr: final_pod_cidr.unwrap(),
        })
    }
}
