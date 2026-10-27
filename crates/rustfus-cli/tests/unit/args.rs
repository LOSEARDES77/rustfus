use super::*;
use clap::CommandFactory;

#[test]
fn cli_is_valid() {
    Cli::command().debug_assert();
}

#[test]
fn fs_args_match_fs_names() {
    for f in Fs::value_variants() {
        let name = f.to_possible_value().unwrap().get_name().to_string();
        assert_eq!(f.name().to_lowercase(), name);
    }
}
