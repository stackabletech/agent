fn main() {
    use stackable_agent::agentconfig::AgentConfig;
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    let target_file = PathBuf::from("documentation/commandline_args.adoc");

    // Unwrap should be fine here, this will currently get called
    let test = AgentConfig::get_documentation().unwrap();
    fs::write(&target_file, test).unwrap();
}
