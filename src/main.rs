// Gibbs MySQL Spyglass
// Copyright (C) 2016 AgilData
//
// This file is part of Gibbs MySQL Spyglass.
//
// Gibbs MySQL Spyglass is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// Gibbs MySQL Spyglass is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with Gibbs MySQL Spyglass.  If not, see <http://www.gnu.org/licenses/>.

#![feature(plugin)]
#![plugin(regex_macros)]

macro_rules! printfl {
    ($($tt:tt)*) => {{
        use std::io::Write;
        print!($($tt)*);
        ::std::io::stdout().flush().ok().expect("flush() fail");
    }}
}

#[macro_use]
extern crate log;
extern crate env_logger;

extern crate hyper;

extern crate time;

extern crate regex;

use std::{env, io, thread};
use std::net::IpAddr;
use std::fmt::Display;

mod util;
use util::{COpts, rd_opt, wr_opt};

mod capture;
use capture::{CAP_FILE, MAX_CAPTURE, clear_cap, set_cap, cap_size, qry_cnt};
use capture::client::schema;
use capture::sniffer::{get_iface_names, sniff};

mod comm;
use comm::upload;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

use std::str;

pub fn ascii_art() -> &'static str {
    str::from_utf8(include_bytes!("GibbsASCII-ShipPlain.txt")).unwrap()
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CLIState {
    Welcome,
    ChkPerms,
    AskKey,
    ChkKey,
    StartConn,
    AskHost,
    ChkHost,
    AskPort,
    ChkPort,
    AskUser,
    ChkUser,
    AskPass,
    ChkPass,
    AskDb,
    ChkDb,
    AskIface,
    ChkIface,
    AskStart,
    ChkStart,
    AskStop,
    ChkStop,
    AskSend,
    ChkSend,
    Quit,
}

use CLIState::*;

extern crate libc;
use libc::geteuid;

fn act_as_root() -> bool { unsafe { geteuid() == 0 } }

fn again(msg: &str, dflt: &Display) {
    printfl!("{}, please try again [{}] ", msg, dflt);
}

fn rnd_mbs(c: usize) -> usize { (c - 1) / 1_048_576 + 1}

fn cli_act(lst: CLIState, inp: &str, opt: &mut COpts) -> CLIState { match lst {
    Welcome => {
        // println!("{}", ascii_art());
        println!("\nWelcome to Gibbs' Spyglass MySQL Traffic Capture Tool. (v{})\n", VERSION);
        cli_act(ChkPerms, "", opt)
    },
    ChkPerms => {
        if act_as_root() {
            println!("Data will be collected to {}/{}",
                     env::current_dir().unwrap().display(), CAP_FILE);
            cli_act(AskKey, "", opt)
        } else {
            println!("Spyglass is not running with needed permissions to help you.");
            println!("Try starting it with `sudo ` in front of it.");
            cli_act(Quit, "", opt)
        }
    },
    AskKey => {
        printfl!("What is your API Key (get one at https://gibbs.agildata.com/)? [{}] ", opt.key);
        ChkKey
    },
    ChkKey => {
        // TODO:  inp.contains(Pattern of Regex here to check for non-hex chars)
        if inp.len() == 40 {
            opt.key = inp.to_owned();
            cli_act(StartConn, "", opt)
        } else if inp.len() != 0 || opt.key.len() != 40 {
            again("Key must be 40 hex characters long", &opt.key);
            ChkKey
        } else {
            cli_act(StartConn, "", opt)
        }
    },
    StartConn => {
        println!("Great! Let's set up your MySQL connection.");
        cli_act(AskHost, "", opt)
    },
    AskHost => {
        printfl!("    What's your MySQL host? [{}] ", opt.host);
        ChkHost
    },
    ChkHost => {
        if inp.len() > 0 {
            match inp.parse::<IpAddr>() {
                Ok(h) => {
                    opt.host = h;
                    cli_act(AskPort, "", opt)
                },
                Err(e) => {
                    again(&e.to_string(), &opt.host);
                    lst
                },
            }
        } else { cli_act(AskPort, "", opt) }
     },
    AskPort => {
        printfl!("       And your MySQL port? [{}] ", opt.port);
        ChkPort
    },
    ChkPort => {
        if inp.len() > 0 {
            match u16::from_str_radix(&inp, 10) {
                Ok(p) => {
                    opt.port = p;
                    cli_act(AskUser, "", opt)
                },
                Err(e) => {
                    again(&e.to_string(), &opt.port);
                    lst
                },
            }
        } else { cli_act(AskUser, "", opt) }
    },
    AskUser => {
        printfl!("       And your MySQL username? [{}] ", opt.user);
        ChkUser
    },
    ChkUser => {
        if inp.len() > 0 { opt.user = inp.to_owned(); }
        cli_act(AskPass, "", opt)
    },
    AskPass => {
        printfl!("       And your MySQL password? [] ");
        ChkPass
    },
    ChkPass => {
        if inp.len() > 0 { opt.pass = inp.to_owned(); }
        cli_act(AskDb, "", opt)
    },
    AskDb => {
        printfl!("       And your MySQL database to analyze? [{}] ", opt.db);
        ChkDb
    },
    ChkDb => {
        if inp.len() > 0 { opt.db = inp.to_owned(); }
        printfl!("\nQuerying schema");
        match schema(opt.clone()) {
            Ok(_) => {
                println!("\nSchema done.\n");
                cli_act(AskIface, "", opt)
            },
            Err(e) => {
                println!("\n{:?}", e);
                cli_act(AskHost, "", opt)
            },
        }
    },
    AskIface => {
        let fs = get_iface_names();
        match fs.len() {
            0 => {
                println!("\n\nNo proper active network interfaces for Spyglass to use! Press enter to complete this run.");
                cli_act(Quit, "", opt)
            },
            _ => {
                opt.iface = fs.get(0).unwrap().to_owned();
                if fs.len() == 1 {
                    cli_act(AskStart, "", opt)
                } else {
                    printfl!("\n    And finally, pick your network interface carrying MySQL traffic? (use one of: {:?}) [{}] ", fs, opt.iface);
                    ChkIface
                }
            },
        }
    },
    ChkIface => {
        if inp.len() > 0 {
            if get_iface_names().contains(&inp.to_string()) {
                opt.iface = inp.to_owned();
                cli_act(AskStart, "", opt)
            } else {
                again("Please enter a valid interface from the list", &opt.iface);
                lst
            }
        } else {
            cli_act(AskStart, "", opt)
        }
    },
    AskStart => {
        printfl!("\nSuper! We're all set. Press enter to start data capture.");
        ChkStart
    },
    ChkStart => {
        set_cap(true);
        cli_act(AskStop, "", opt)
    },
    AskStop => {
        printfl!("Starting capture, will auto-stop after {} MB of data, or press enter to stop.",
                 rnd_mbs(MAX_CAPTURE));
        let sniff_opt = opt.clone();
        let _= thread::spawn(|| {
            sniff(sniff_opt);
        });
        ChkStop
    },
    ChkStop => {
        set_cap(false);
        println!("\nData capture stopped. We found {} queries, totaling {:#} MB of data.",
                 qry_cnt(), rnd_mbs(cap_size()));
        cli_act(AskSend, "", opt)
    },
    AskSend => {
        printfl!("Would you like to upload {} to Gibbs now? [y] ", CAP_FILE);
        ChkSend
    },
    ChkSend => {
        if inp.len() == 0 || inp.to_string().to_uppercase() == "Y" {
            printfl!("\nSending......");
            match upload(opt.clone()) {
                Some(a) => {
                    println!(".done.");
                    println!("\nYou can check on the status of your analysis by going to this URL: https://gibbs.agildata.com/analyses/{}", a);
                },
                None => {
                    println!(".failed!");
                    println!("\nSomething prevented the file {}/{} from uploading.",
                             env::current_dir().unwrap().display(), CAP_FILE);
                    println!("See if you can send it using this URL: https://gibbs.agildata.com/manualUpload");
                },
            }
        }
        cli_act(Quit, "", opt)
    },
    Quit => {
        println!("Spyglass done! Press enter to complete this run. ");
        Quit
    },
} }

fn main() {
    env::set_var("RUST_BACKTRACE", "1");
    let _ = env_logger::init();

    clear_cap();

    let mut st: CLIState = Welcome;
    let mut inp = String::new();
    let mut opt = rd_opt();

    while st != Quit {
        st = cli_act(st, &inp, &mut opt);
        inp.clear();
        match io::stdin().read_line(&mut inp) {
            Ok(_) => { inp.pop(); },
            Err(e) => again(&e.to_string(), &""),
        }
    }

    wr_opt(opt);
}
