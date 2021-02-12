/// This is a helper binary which generates the file `documentation/commandline_args.adoc` which
/// contains documentation of the available command line options for the agent binary.
///
/// It gets the content by calling [`stackable_agent::config::AgentConfig::get_documentation()`]
///
/// # Panics
/// This will panic if an error occurs when trying to write the file.

fn main() {
    use stackable_agent::config::AgentConfig;
    use std::fs;

    let target_file = "documentation/commandline_args.adoc";
    fs::write(target_file, AgentConfig::get_documentation()).unwrap_or_else(|err| {
        panic!(
            "Could not write documentation to [{}]: {}",
            target_file, err
        )
    });
}
