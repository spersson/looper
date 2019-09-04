use looper::{Child, Core, ObjectId};
use std::io::Read;
use std::process::Command;

// Tests running commands in sequence.

struct Sequence {
    id: ObjectId,
    child: Child<()>,
}

impl Sequence {
    fn read(&mut self, _core: &mut Core) {
        let mut s = String::new();
        match self.child.stdout.read_to_string(&mut s) {
            Ok(_) => {
                if !s.is_empty() {
                    eprintln!("output: {}", s)
                }
            }
            Err(e) => eprintln!("Error reading from child: {}", e),
        }
    }

    fn handle_death_1(&mut self, core: &mut Core) {
        eprintln!("handling death 1");
        let echo = core
            .spawn(Command::new("echo").arg("papapapapapap"))
            .expect("echo executable must exist.")
            .close_stdin();
        core.register_reaper(&echo, self.id, Sequence::handle_death_2);
        core.register_reader(&echo.stdout, self.id, Sequence::read);
        self.child = echo;
    }

    fn handle_death_2(&mut self, core: &mut Core) {
        core.exit();
        eprintln!("called exit.");
    }
}

fn main() {
    let mut core = Core::new();
    let e1 = core
        .spawn(Command::new("echo").arg("mamamamamam"))
        .expect("echo executable must exist.")
        .close_stdin();
    let id = core.next_id();
    core.register_reaper(&e1, id, Sequence::handle_death_1);
    core.register_reader(&e1.stdout, id, Sequence::read);
    core.add(Sequence { child: e1, id });
    core.run();
}
