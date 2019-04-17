use looper_core::{Child, Core};
use std::io::{self, Read};
use std::process::Command;

struct State {
    child: Child,
}

impl State {
    fn handle_death(&mut self, core: &mut Core) {
        eprintln!("I smell death.");
        core.exit();
    }

    fn read_stdout(&mut self, _core: &mut Core) {
        eprintln!("about to read.");
        let mut buffer = String::new();
        self.child.stdout.read_to_string(&mut buffer).unwrap();
        dbg!(buffer);
    }
}

fn main() -> io::Result<()> {
    let mut core = Core::new();
    let child = Child::new(Command::new("echo").arg("Hello there."))?;
    let id = core.next_id();
    core.register_reader(&child.stdout, id, State::read_stdout);
    core.register_reaper(&child, id, State::handle_death);
    core.add(Box::new(State { child }));
    core.run();
    Ok(())
}
