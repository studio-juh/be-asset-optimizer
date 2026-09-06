fn main() {
    std::process::exit(be_asset_optimizer::cli::run(std::env::args_os().skip(1)));
}
