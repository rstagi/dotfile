use std::{fs, path::Path};

pub fn copy(name: &str, destination: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ws-daemon/src/import/fixtures")
        .join(name);
    copy_tree(&source, destination, destination);
}

fn copy_tree(source: &Path, destination: &Path, root: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target, root);
        } else {
            let bytes = fs::read(entry.path()).unwrap();
            let text = String::from_utf8(bytes).unwrap();
            fs::write(target, text.replace("@WS_HOME@", root.to_str().unwrap())).unwrap();
        }
    }
}
