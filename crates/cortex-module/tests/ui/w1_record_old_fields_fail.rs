// Wave 1 ports (b): a record is immutable. It has no `revision`, no `title` and no `updated_at`: the title is
// the `summary`, and `created_at` replaces `updated_at`.
use cortex_module::RecordView;

fn old(v: &RecordView) {
    let _ = v.revision;
    let _ = &v.title;
    let _ = v.updated_at;
}

fn main() {
    let _ = old;
}
