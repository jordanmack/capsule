mod checker;
mod config;
mod debugger;
mod deployment;
mod generator;
mod project_context;
mod recipe;
mod signal;
mod tester;
mod util;
mod wallet;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::exit;
use std::str::FromStr;

use anyhow::{anyhow, Result};
use checker::Checker;
use ckb_tool::ckb_types::core::Capacity;
use deployment::manage::{DeployOption, Manage as DeployManage};
use generator::new_project;
use project_context::{load_project_context, BuildEnv, DeployEnv};
use recipe::get_recipe;
use tester::Tester;
use wallet::cli_types::HumanCapacity;
use wallet::{Address, Wallet, DEFAULT_CKB_CLI_BIN_NAME, DEFAULT_CKB_RPC_URL};

use clap::{App, Arg, SubCommand};

fn run_cli() -> Result<()> {
    env_logger::init();
    let matches = App::new("Capsule")
        .version("0.0.0-pre.1")
        .author("Nervos Developer Tools Team")
        .about("Capsule CKB contract scaffold")
        .subcommand(SubCommand::with_name("check").about("Check environment and dependencies").display_order(0))
        .subcommand(SubCommand::with_name("new").about("Create a new project").arg(Arg::with_name("name").help("project name").index(1).required(true).takes_value(true)).display_order(1))
        .subcommand(SubCommand::with_name("build").about("Build contracts").arg(Arg::with_name("name").short("n").long("name").multiple(true).takes_value(true).help("contract name")).arg(
                    Arg::with_name("release").long("release").help("Build contracts in release mode.")
        ).display_order(2))
        .subcommand(SubCommand::with_name("run").about("Run command in contract build image").usage("capsule run --name <name> 'echo list contract dir: && ls'")
        .args(&[Arg::with_name("name").short("n").long("name").required(true).takes_value(true).help("contract name"),
                Arg::with_name("cmd").required(true).multiple(true).help("command to run")])
        .display_order(3))
        .subcommand(SubCommand::with_name("test").about("Run tests").arg(
                    Arg::with_name("release").long("release").help("Test release mode contracts.")
        ).display_order(4))
        .subcommand(
            SubCommand::with_name("deploy")
                .about("Deploy contracts, edit deployment.toml to custodian deployment recipe.")
                .args(&[
                    Arg::with_name("address").long("address").help(
                        "Denote which address provides cells",
                    ).required(true).takes_value(true),
                    Arg::with_name("fee").long("fee").help(
                        "Per transaction's fee, deployment may involve more than one transaction.",
                    ).default_value("0.0001").takes_value(true),
                    Arg::with_name("env").long("env").help("Deployment environment.")
                    .possible_values(&["dev", "production"]).default_value("dev").takes_value(true),
                    Arg::with_name("migrate")
                        .long("migrate")
                        .help("Use previously deployed cells as inputs.").possible_values(&["on", "off"]).default_value("on").takes_value(true),
                    Arg::with_name("api")
                        .long("api")
                        .help("CKB RPC url").default_value(DEFAULT_CKB_RPC_URL).takes_value(true),
                    Arg::with_name("ckb-cli")
                        .long("ckb-cli")
                        .help("CKB cli binary").default_value(DEFAULT_CKB_CLI_BIN_NAME).takes_value(true),
                ]).display_order(5),
        )
        .subcommand(
            SubCommand::with_name("debugger")
            .about("GDB debugger server")
            .subcommand(
                SubCommand::with_name("gen-template")
                .about("Generate transaction debugging template")
                .args(&[
                    Arg::with_name("contract-path").long("contract-path").short("p").help(
                        "Binary contract path",
                    ).required(true).takes_value(true),
                    Arg::with_name("output-file")
                        .long("output-file")
                        .short("o")
                        .help("Output file path").required(true).takes_value(true),
                ])
            )
            .subcommand(
                SubCommand::with_name("start")
                .about("Start GDB")
                .args(&[
                    Arg::with_name("template-file")
                        .long("template-file")
                        .short("f")
                        .help("Transaction debugging template file")
                        .required(true)
                        .takes_value(true),
                    Arg::with_name("name")
                        .short("n")
                        .long("name")
                        .required(true)
                        .takes_value(true)
                        .help("contract name"),
                    Arg::with_name("script-group-type")
                        .long("script-group-type")
                        .possible_values(&["type", "lock"])
                        .help("Script type")
                        .required(true)
                        .takes_value(true),
                    Arg::with_name("cell-index")
                        .long("cell-index")
                        .required(true)
                        .help("index of the cell")
                        .takes_value(true),
                    Arg::with_name("cell-type")
                        .long("cell-type")
                        .required(true)
                        .possible_values(&["input", "output"])
                        .help("cell type")
                        .takes_value(true),
                    Arg::with_name("listen")
                        .long("listen")
                        .short("l")
                        .help("GDB server listening port")
                        .default_value("8000").required(true).takes_value(true),
                ])
            )
                .display_order(6),
        )
        .get_matches();
    let signal = signal::Signal::setup();
    match matches.subcommand() {
        ("check", _args) => {
            Checker::build()?.print_report();
        }
        ("new", Some(args)) => {
            let mut name = args
                .value_of("name")
                .expect("name")
                .trim()
                .trim_end_matches("/")
                .to_string();
            let mut path = PathBuf::new();
            if let Some(index) = name.rfind("/") {
                path.push(&name[..index]);
                name = name[index + 1..].to_string();
            } else {
                path.push(env::current_dir()?);
            }
            new_project(name.to_string(), path, &signal)?;
        }
        ("build", Some(args)) => {
            let context = load_project_context()?;
            let build_names: Vec<&str> = args
                .values_of("name")
                .map(|values| values.collect())
                .unwrap_or_default();
            let build_env: BuildEnv = if args.is_present("release") {
                BuildEnv::Release
            } else {
                BuildEnv::Debug
            };
            let contracts: Vec<_> = context
                .config
                .contracts
                .iter()
                .filter(|c| build_names.is_empty() || build_names.contains(&c.name.as_str()))
                .collect();
            if contracts.is_empty() {
                println!("Nothing to do");
            } else {
                for c in contracts {
                    println!("Building contract {}", c.name);
                    get_recipe(&context, c)?.run_build(build_env, &signal)?;
                }
                println!("Done");
            }
        }
        ("run", Some(args)) => {
            let context = load_project_context()?;
            let name = args.value_of("name").expect("name");
            let cmd = args
                .values_of("cmd")
                .expect("cmd")
                .collect::<Vec<&str>>()
                .join(" ");
            let contract = match context.config.contracts.iter().find(|c| name == c.name) {
                Some(c) => c,
                None => return Err(anyhow!("can't find contract '{}'", name)),
            };
            get_recipe(&context, contract)?.run(cmd, &signal)?;
        }
        ("test", Some(args)) => {
            let context = load_project_context()?;
            let build_env: BuildEnv = if args.is_present("release") {
                BuildEnv::Release
            } else {
                BuildEnv::Debug
            };
            Tester::run(&context, build_env, &signal)?;
        }
        ("deploy", Some(args)) => {
            if !Checker::build()?.ckb_cli {
                eprintln!("Can't find ckb-cli, install it to enable deployment");
                exit(1);
            }
            let address = {
                let address_hex = args.value_of("address").expect("address");
                Address::from_str(&address_hex).expect("parse address")
            };
            let context = load_project_context()?;
            let ckb_rpc_url = args.value_of("api").expect("api");
            let ckb_cli_bin = args.value_of("ckb-cli").expect("ckb-cli");
            let wallet = Wallet::load(ckb_rpc_url.to_string(), ckb_cli_bin.to_string(), address);
            let deploy_env: DeployEnv = args
                .value_of("env")
                .expect("deploy env")
                .parse()
                .map_err(|err: &str| anyhow!(err))?;
            let migration_dir = context.migrations_path(deploy_env);
            let migrate = args.value_of("migrate").expect("migrate").to_lowercase() == "on";
            let tx_fee = match HumanCapacity::from_str(args.value_of("fee").expect("tx fee")) {
                Ok(tx_fee) => Capacity::shannons(tx_fee.0),
                Err(err) => return Err(anyhow!(err)),
            };
            let opt = DeployOption { migrate, tx_fee };
            DeployManage::new(migration_dir, context.load_deployment()?).deploy(wallet, opt)?;
        }
        ("debugger", Some(sub_matches)) => match sub_matches.subcommand() {
            ("gen-template", Some(args)) => {
                let contract_path = args.value_of("contract-path").expect("contract path");
                let mock_tx = debugger::build_template(contract_path)?;
                let template_path = args.value_of("output-file").expect("output file");
                let mock_tx: debugger::transaction::ReprMockTransaction = mock_tx.into();
                fs::write(&template_path, serde_json::to_string(&mock_tx)?)?;
                println!("Write transaction debugging template to {}", template_path,);
            }
            ("start", Some(args)) => {
                let context = load_project_context()?;
                let template_path = args.value_of("template-file").expect("template file");
                let contract_name = args.value_of("name").expect("contract name");
                let script_group_type = args.value_of("script-group-type").unwrap();
                let cell_index: usize = args.value_of("cell-index").unwrap().parse()?;
                let cell_type = args.value_of("cell-type").unwrap();
                let listen_port: usize = args
                    .value_of("listen")
                    .unwrap()
                    .parse()
                    .expect("listen port");
                debugger::start_debugger(
                    &context,
                    template_path,
                    contract_name,
                    script_group_type,
                    cell_index,
                    cell_type,
                    listen_port,
                    true,
                    &signal,
                )?;
            }
            (command, _) => {
                panic!("unknown debugger subcommand '{}'", command);
            }
        },
        (command, _) => {
            eprintln!("unrecognize command '{}'", command);
            exit(1);
        }
    }
    Ok(())
}

fn main() {
    run_cli().expect("error");
}
