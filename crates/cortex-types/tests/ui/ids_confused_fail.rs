// F1-04: a WorkerId cannot go where a ProjectId is expected.
use cortex_types::{ProjectId, WorkerId};

fn wants_project(_: ProjectId) {}

fn main() {
    let _pass_worker = |w: WorkerId| wants_project(w);
}
