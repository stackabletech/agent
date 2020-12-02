use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;

use log::{debug, error, info, trace};
use pnet::datalink;
use stackable_config::{ConfigOption, Configurable, Configuration};
use thiserror::Error;

use crate::agentconfig::AgentConfigError::WrongArgumentCount;

#[derive(Error, Debug)]
pub enum AgentConfigError {
    #[error("Wrong number of arguments found for config option {}!", .option.name)]
    WrongArgumentCount { option: ConfigOption },
}

#[derive(Clone)]
pub struct AgentConfig {
    pub parcel_directory: PathBuf,
    pub config_directory: PathBuf,
    pub log_directory: PathBuf,
    pub server_ip_address: IpAddr,
    pub server_cert_file: Option<PathBuf>,
    pub server_key_file: Option<PathBuf>,
    pub tags: HashMap<String, String>,
}

impl AgentConfig {
    pub const SERVER_IP_ADDRESS: ConfigOption = ConfigOption {
        name: "server-bind-ip",
        default: None,
        required: false,
        takes_argument: true,
        help: "The local IP to register as the node's ip with the apiserver. Will be automatically set to the first address of the first non-loopback interface if not specified.",
        documentation: "",
        list: false,
    };

    pub const SERVER_CERT_FILE: ConfigOption = ConfigOption {
        name: "server-cert-file",
        default: None,
        required: false,
        takes_argument: true,
        help: "The certificate file for the local webserver which the Krustlet starts.",
        documentation: "",
        list: false,
    };

    pub const SERVER_KEY_FILE: ConfigOption = ConfigOption {
        name: "server-key-file",
        default: None,
        required: false,
        takes_argument: true,
        help:
            "Private key file (in PKCS8 format) to use for the local webserver the Krustlet starts.",
        documentation: "",
        list: false,
    };

    pub const PACKAGE_DIR: ConfigOption = ConfigOption {
        name: "package-directory",
        default: Some("/opt/stackable/packages"),
        required: false,
        takes_argument: true,
        help: "The base directory under which installed packages will be stored.",
        documentation: "This directory will serve as starting point for packages that are needed by \
        pods assigned to this node.\n Packages will be downloaded into the \"_download\" folder at the
top level of this folder as archives and remain there for potential future use.\n\
        Archives will the be extracted directly into this folder in subdirectories following the naming
scheme of \"productname-productversion\".
        The agent will need full access to this directory and tries to create it if it does not exist.",
        list: false,
    };

    pub const CONFIG_DIR: ConfigOption = ConfigOption {
        name: "config-directory",
        default: Some("/opt/stackable/config"),
        required: false,
        takes_argument: true,
        help: "The base directory under which configuration will be generated for all executed services.",
        documentation: "This directory will serve as starting point for all log files which this service creates.\
        Every service will get its own subdirectories created within this directory - for every service start a \
        new subdirectory will be created to show a full history of configuration that was used for this service.\n
        ConfigMaps that are mounted into the pod that describes this service will be created relative to these run \
        directories - unless the mounts specify an absolute path, in which case it is allowed to break out of this directory.\n\n\
        The agent will need full access to this directory and tries to create it if it does not exist.",        
        list: false,
    };

    pub const LOG_DIR: ConfigOption = ConfigOption {
        name: "log-directory",
        default: Some("/opt/stackable/logs"),
        required: false,
        takes_argument: true,
        help: "The base directory under which log files will be placed for all services.",
        documentation: "This directory will serve as starting point for all log files which this service creates.\
        Every service will get its own subdirectory created within this directory.\n
        Anything that is then specified in the log4j config or similar files will be resolved relatively to this directory.\n\n\
        The agent will need full access to this directory and tries to create it if it does not exist.",
        list: false,
    };

    pub const NO_CONFIG: ConfigOption = ConfigOption {
        name: "no-config",
        default: None,
        required: false,
        takes_argument: false,
        help: "If this option is specified, any file referenced in AGENT_CONF environment variable will be ignored.",
        documentation: "",
        list: false,
    };

    pub const TAG: ConfigOption = ConfigOption {
        name: "tag",
        default: None,
        required: false,
        takes_argument: true,
        help: "A \"key=value\" pair that should be assigned to this agent as tag. This can be specified multiple times to assign additional tags.",
        documentation: "Tags are the main way of identifying nodes to assign services to later on.",
        list: true
    };

    fn get_options() -> HashSet<ConfigOption> {
        [
            AgentConfig::SERVER_IP_ADDRESS,
            AgentConfig::SERVER_CERT_FILE,
            AgentConfig::SERVER_KEY_FILE,
            AgentConfig::PACKAGE_DIR,
            AgentConfig::CONFIG_DIR,
            AgentConfig::LOG_DIR,
            AgentConfig::NO_CONFIG,
            AgentConfig::TAG,
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
            if list_value.len() != 1 {
                error!("Got additional, unexpected values for parameter!");
            } else {
                // We've checked that the list has exactly one value at this point, so no errors should
                // occur after this point - but you never know
                return Ok(list_value[0].to_string());
            }
        }
        Err(WrongArgumentCount {
            option: option.clone(),
        })
    }

    /// This tries to find the first non loopback interface with an ip address assigned.
    /// This should usually be the default interface according to:
    ///
    /// https://docs.rs/pnet/0.27.2/pnet/datalink/fn.interfaces.html
    fn get_default_ipaddress() -> Option<IpAddr> {
        let all_interfaces = datalink::interfaces();

        let default_interface = all_interfaces
            .iter()
            .find(|e| e.is_up() && !e.is_loopback() && !e.ips.is_empty());

        match default_interface {
            Some(interface) => {
                debug!(
                    "Found default interface {} with following ips: [{:?}].",
                    interface, interface.ips
                );
                if let Some(ipv4_network) = interface.ips.get(0) {
                    return Some(ipv4_network.ip());
                }
            }
            None => error!(
                "Error while finding the default interface - delegating ip retrieval to Kubelet."
            ),
        };
        None
    }
}

impl Configurable for AgentConfig {
    fn get_config_description() -> Configuration {
        Configuration {
            name: "Stackable Agent",
            version: "0.1",
            about: "Manages local state according to what the central orchestrator defines.",
            options: AgentConfig::get_options(),
        }
    }

    fn parse_values(
        parsed_values: HashMap<ConfigOption, Option<Vec<String>>, RandomState>,
    ) -> Result<Self, anyhow::Error> {
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

        // Parse log directory
        let final_log_dir = if let Ok(log_dir) =
            AgentConfig::get_exactly_one_string(&parsed_values, &AgentConfig::LOG_DIR)
        {
            PathBuf::from_str(&log_dir).unwrap_or_else(|_| {
                panic!("Error parsing valid log directory from string: {}", log_dir)
            })
        } else {
            PathBuf::from_str(
                AgentConfig::LOG_DIR
                    .default
                    .expect("Invalid default value for log directory option!"),
            )
            .unwrap_or_else(|_| panic!("Unable to get log directory from options!".to_string()))
        };

        // Parse config directory
        let final_config_dir = if let Ok(config_dir) =
            AgentConfig::get_exactly_one_string(&parsed_values, &AgentConfig::CONFIG_DIR)
        {
            PathBuf::from_str(&config_dir).unwrap_or_else(|_| {
                panic!(
                    "Error parsing valid config directory from string: {}",
                    config_dir
                )
            })
        } else {
            PathBuf::from_str(
                AgentConfig::CONFIG_DIR
                    .default
                    .expect("Invalid default value for config directory option!"),
            )
            .unwrap_or_else(|_| panic!("Unable to get config directory from options!".to_string()))
        };

        // Parse parcel directory
        let final_parcel_dir = if let Ok(parcel_dir) =
            AgentConfig::get_exactly_one_string(&parsed_values, &AgentConfig::PACKAGE_DIR)
        {
            PathBuf::from_str(&parcel_dir).unwrap_or_else(|_| {
                panic!(
                    "Error parsing valid parcel directory from string: {}",
                    parcel_dir
                )
            })
        } else {
            PathBuf::from_str(
                AgentConfig::PACKAGE_DIR
                    .default
                    .expect("Invalid default value for parcel directory option!"),
            )
            .unwrap_or_else(|_| panic!("Unable to get parcel directory from options!".to_string()))
        };

        // Parse cert file
        let final_server_cert_file = if let Ok(server_cert_file) =
            AgentConfig::get_exactly_one_string(&parsed_values, &AgentConfig::SERVER_CERT_FILE)
        {
            Some(PathBuf::from_str(&server_cert_file).unwrap_or_else(|_| {
                panic!(
                    "Error parsing valid server cert file directory from string: {}",
                    server_cert_file
                )
            }))
        } else {
            None
        };

        // Parse key file
        let final_server_key_file = if let Ok(server_key_file) =
            AgentConfig::get_exactly_one_string(&parsed_values, &AgentConfig::SERVER_KEY_FILE)
        {
            Some(PathBuf::from_str(&server_key_file).unwrap_or_else(|_| {
                panic!(
                    "Error parsing valid server key file directory from string: {}",
                    server_key_file
                )
            }))
        } else {
            None
        };

        let mut final_tags: HashMap<String, String> = HashMap::new();
        if let Some(Some(tags)) = parsed_values.get(&AgentConfig::TAG) {
            for tag in tags {
                let split: Vec<&str> = tag.split('=').collect();
                if split.len() != 2 {
                    // We want to avoid any "unpredictable" behavior like ignoring a malformed
                    // key=value pair with just a log message -> so we panic if this can't be
                    // parsed
                    panic!(format!(
                        "Unable to parse value [{}] for option --tag as key=value pair!",
                        tag
                    ))
                } else {
                    // This might panic, but really shouldn't, as we've checked the size of the array
                    final_tags.insert(split[0].to_string(), split[1].to_string());
                }
            }
        }

        Ok(AgentConfig {
            parcel_directory: final_parcel_dir,
            config_directory: final_config_dir,
            log_directory: final_log_dir,
            server_ip_address: final_ip,
            server_cert_file: final_server_cert_file,
            server_key_file: final_server_key_file,
            tags: final_tags,
        })
    }
}
