use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Loading,
    Done,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageItem {
    pub id: u64,
    pub path: PathBuf,
    pub file_name: String,
    pub status: Status,
    /// Tinify or filesystem error, or a note when the original could not be trashed.
    pub message: Option<String>,
}

#[derive(Debug, Default)]
pub struct Images {
    items: Vec<ImageItem>,
    next_id: u64,
}

impl Images {
    pub fn iter(&self) -> impl Iterator<Item = &ImageItem> {
        self.items.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn any_loading(&self) -> bool {
        self.items.iter().any(|item| item.status == Status::Loading)
    }

    /// Queue `path` for upload.
    ///
    /// Returns the id to upload. A path already uploading is left alone and
    /// returns `None`. A path that finished is reset and uploaded again.
    pub fn add(&mut self, path: PathBuf) -> Option<u64> {
        if let Some(existing) = self.items.iter_mut().find(|item| item.path == path) {
            if existing.status == Status::Loading {
                return None;
            }
            existing.status = Status::Loading;
            existing.message = None;
            return Some(existing.id);
        }

        self.next_id += 1;
        let id = self.next_id;
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        self.items.push(ImageItem {
            id,
            path,
            file_name,
            status: Status::Loading,
            message: None,
        });
        Some(id)
    }

    pub fn mark_done(&mut self, id: u64, message: Option<String>) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.status = Status::Done;
            item.message = message;
        }
    }

    pub fn mark_error(&mut self, id: u64, message: String) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.status = Status::Error;
            item.message = Some(message);
        }
    }

    /// Drop a finished image. Loading images stay until the upload ends.
    pub fn remove(&mut self, id: u64) {
        self.items
            .retain(|item| item.id != id || item.status == Status::Loading);
    }

    pub fn remove_all(&mut self) {
        self.items.retain(|item| item.status == Status::Loading);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    #[test]
    fn second_add_while_loading_is_ignored() {
        let mut images = Images::default();
        let id = images.add(path("/a.png")).unwrap();
        assert!(images.add(path("/a.png")).is_none());
        assert_eq!(images.iter().count(), 1);
        assert_eq!(images.iter().next().unwrap().id, id);
        assert_eq!(images.iter().next().unwrap().status, Status::Loading);
    }

    #[test]
    fn finished_image_can_be_uploaded_again() {
        let mut images = Images::default();
        let id = images.add(path("/a.png")).unwrap();
        images.mark_done(id, None);
        images.mark_error(id, "nope".into());
        let again = images.add(path("/a.png")).unwrap();
        assert_eq!(again, id);
        let item = images.iter().next().unwrap();
        assert_eq!(item.status, Status::Loading);
        assert_eq!(item.message, None);
    }

    #[test]
    fn remove_keeps_loading_images() {
        let mut images = Images::default();
        let loading = images.add(path("/loading.png")).unwrap();
        let done = images.add(path("/done.png")).unwrap();
        images.mark_done(done, None);
        images.remove(loading);
        images.remove(done);
        let left: Vec<_> = images.iter().map(|item| item.id).collect();
        assert_eq!(left, vec![loading]);
    }

    #[test]
    fn remove_all_keeps_only_loading_images() {
        let mut images = Images::default();
        let loading = images.add(path("/loading.png")).unwrap();
        let done = images.add(path("/done.png")).unwrap();
        let failed = images.add(path("/bad.png")).unwrap();
        images.mark_done(done, None);
        images.mark_error(failed, "Tinify".into());
        images.remove_all();
        let left: Vec<_> = images.iter().map(|item| item.id).collect();
        assert_eq!(left, vec![loading]);
    }

    #[test]
    fn unknown_id_is_ignored() {
        let mut images = Images::default();
        images.mark_done(4, Some("x".into()));
        images.mark_error(4, "x".into());
        images.remove(4);
        assert!(images.is_empty());
    }
}
