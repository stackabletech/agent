use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
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
        help: "The local IP to bind the callback webserver to. Will be automatically set to the first address of the first non-loopback interface if not specified.",
        documentation: "",
        list: false,
    };

    pub const SERVER_KEY_FILE: ConfigOption = ConfigOption {
        name: "server-key-file",
        default: None,
        required: false,
        takes_argument: true,
        help: "The local IP to bind the callback webserver to. Will be automatically set to the first address of the first non-loopback interface if not specified.",
        documentation: "",
        list: false,
    };

    pub const PARCEL_DIR: ConfigOption = ConfigOption {
        name: "parcel-directory",
        default: Some("/opt/stackable/parcels"),
        required: false,
        takes_argument: true,
        help: "The base directory under which installed parcels will be stored.",
        documentation: "Yak Yak!",
        list: false,
    };

    pub const CONFIG_DIR: ConfigOption = ConfigOption {
        name: "config-directory",
        default: Some("/opt/stackable/config"),
        required: false,
        takes_argument: true,
        help: "The base directory under which configuration will be generated for all executed services.",
        documentation: "Yak Yak!",
        list: false,
    };

    pub const LOG_DIR: ConfigOption = ConfigOption {
        name: "log-directory",
        default: Some("/opt/stackable/logs"),
        required: false,
        takes_argument: true,
        help: "The base directory under which log files will be placed for all services.",
        documentation: "Yak Yak!",
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
            AgentConfig::PARCEL_DIR,
            AgentConfig::CONFIG_DIR,
            AgentConfig::LOG_DIR,
            AgentConfig::NO_CONFIG,
            AgentConfig::TAG,
        ]
        .iter()
        .cloned()
        .collect()
    }

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
                //panic!(&format!("Expected exactly one value to be specified for parameter {} but got {} instead.", option.name.clone(), list_value.len().clone()));
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

    fn get_at_least_one_string(
        parsed_values: &HashMap<ConfigOption, Option<Vec<String>>>,
        option: &ConfigOption,
    ) -> Vec<String> {
        if let Some(Some(list_value)) = parsed_values.get(option) {
            if list_value.len() > 0 {
                return list_value.clone();
            }
            panic!("Unexpectedly got empty list of values for parameter".to_string());
        } else {
            panic!("Parameter was not specified but a value is required!".to_string());
        }
    }

    fn get_default_ipaddress() -> Option<IpAddr> {
        let all_interfaces = datalink::interfaces();

        let default_interface = all_interfaces
            .iter()
            .filter(|e| e.is_up() && !e.is_loopback() && e.ips.len() > 0)
            .next();

        match default_interface {
            Some(interface) => {
                debug!(
                    "Found default interface {} with following ips: [{:?}].",
                    interface, interface.ips
                );
                if let ipv4_network = interface.ips[0] {
                    return Some(ipv4_network.ip());
                }
            }
            None => error!(
                "Error while finding the default interface - delegating ip retrieval to Kubelet."
            ),
        };
        return None;
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
    ) -> Result<Box<Self>, anyhow::Error> {
        // Parse IP Address or lookup default
        let final_ip = if let Ok(ip) =
            AgentConfig::get_exactly_one_string(&parsed_values, &AgentConfig::SERVER_IP_ADDRESS)
        {
            IpAddr::from_str(&ip).expect(&format!("Couldn't parse {} as a valid ip address!", ip))
        } else {
            AgentConfig::get_default_ipaddress().expect(&format!(
                "Error getting default ip address, please specify it explicitly!"
            ))
        };
        info!("Selected {} as local address to listen on.", final_ip);

        // Parse log directory
        let final_log_dir = if let Ok(log_dir) =
            AgentConfig::get_exactly_one_string(&parsed_values, &AgentConfig::LOG_DIR)
        {
            PathBuf::from_str(&log_dir).expect(&format!(
                "Error parsing valid log directory from string: {}",
                log_dir
            ))
        } else {
            PathBuf::from_str(
                AgentConfig::LOG_DIR
                    .default
                    .expect("Invalid default value for log directory option!"),
            )
            .expect(&format!("Unable to get log directory from options!"))
        };

        // Parse config directory
        let final_config_dir = if let Ok(config_dir) =
            AgentConfig::get_exactly_one_string(&parsed_values, &AgentConfig::CONFIG_DIR)
        {
            PathBuf::from_str(&config_dir).expect(&format!(
                "Error parsing valid config directory from string: {}",
                config_dir
            ))
        } else {
            PathBuf::from_str(
                AgentConfig::CONFIG_DIR
                    .default
                    .expect("Invalid default value for config directory option!"),
            )
            .expect(&format!("Unable to get config directory from options!"))
        };

        // Parse parcel directory
        let final_parcel_dir = if let Ok(parcel_dir) =
            AgentConfig::get_exactly_one_string(&parsed_values, &AgentConfig::PARCEL_DIR)
        {
            PathBuf::from_str(&parcel_dir).expect(&format!(
                "Error parsing valid parcel directory from string: {}",
                parcel_dir
            ))
        } else {
            PathBuf::from_str(
                AgentConfig::PARCEL_DIR
                    .default
                    .expect("Invalid default value for parcel directory option!"),
            )
            .expect(&format!("Unable to get parcel directory from options!"))
        };

        // Parse cert file
        let final_server_cert_file = if let Ok(server_cert_file) =
            AgentConfig::get_exactly_one_string(&parsed_values, &AgentConfig::SERVER_CERT_FILE)
        {
            Some(PathBuf::from_str(&server_cert_file).expect(&format!(
                "Error parsing valid server cert file directory from string: {}",
                server_cert_file
            )))
        } else {
            None
        };

        // Parse key file
        let final_server_key_file = if let Ok(server_key_file) =
            AgentConfig::get_exactly_one_string(&parsed_values, &AgentConfig::SERVER_KEY_FILE)
        {
            Some(PathBuf::from_str(&server_key_file).expect(&format!(
                "Error parsing valid server key file directory from string: {}",
                server_key_file
            )))
        } else {
            None
        };

        let mut final_tags: HashMap<String, String> = HashMap::new();
        if let Some(Some(tags)) = parsed_values.get(&AgentConfig::TAG) {
            for tag in tags {
                let split: Vec<&str> = tag.split("=").collect();
                if split.len() != 2 {
                    // We want to avoid any "unpredictable" behavior like ignoring a malformed
                    // key=value pair with just a log message -> so we panic if this can't be
                    // parsed
                    panic!(format!(
                        "Unable to parse value {} for option --tag as key=value pair!",
                        tag
                    ))
                } else {
                    // This might panic, but really shouldn't, as we've checked the size of the array
                    final_tags.insert(split[0].to_string(), split[1].to_string());
                }
            }
        }

        Ok(Box::new(AgentConfig {
            parcel_directory: final_parcel_dir,
            config_directory: final_config_dir,
            log_directory: final_log_dir,
            server_ip_address: final_ip,
            server_cert_file: final_server_cert_file,
            server_key_file: final_server_key_file,
            tags: final_tags,
        }))
    }
}
