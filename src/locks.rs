use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

pub struct LockHolder {
    atomic: Arc<AtomicBool>,
}
impl Drop for LockHolder {
    fn drop(&mut self) {
        self.atomic.store(false, Ordering::SeqCst);
    }
}

pub struct Lock {
    sync_locks: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    write_locks: Arc<Mutex<BTreeMap<String, Mutex<()>>>>,
}
impl Lock {
    pub fn new() -> Self {
        Lock {
            sync_locks: Arc::new(Mutex::new(BTreeMap::new())),
            write_locks: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    fn lock<'a>(
        map: &'a Arc<Mutex<BTreeMap<String, Mutex<()>>>>,
        repo_name: &str,
    ) -> MutexGuard<'a, ()> {
        let lock;
        let mut map = map.lock().unwrap();
        if let Some(res_lock) = map.get(repo_name) {
            lock = res_lock;
        } else {
            let new_mutex = Mutex::new(());
            map.insert(repo_name.into(), new_mutex);
            lock = map.get(repo_name).unwrap();
        }

        //allow to move out the lock guard without dropping it
        //we are telling the compiler: ignore the lock guard lifetime
        //this only holds as long as no entry is removed from the map
        unsafe { std::mem::transmute(lock.lock().unwrap()) }
    }

    fn try_lock(
        map: &Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
        repo_name: &str,
    ) -> Option<LockHolder> {
        let mut map = map.lock().unwrap();
        if let Some(atomic) = map.get(repo_name) {
            if atomic
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                Some(LockHolder {
                    atomic: atomic.clone(),
                })
            } else {
                None
            }
        } else {
            let new_mutex = Arc::new(AtomicBool::new(true));
            let holder = LockHolder {
                atomic: new_mutex.clone(),
            };
            map.insert(repo_name.into(), new_mutex);
            Some(holder)
        }
    }

    pub fn lock_sync(&self, repo_name: &str) -> Option<LockHolder> {
        Lock::try_lock(&self.sync_locks, repo_name)
    }

    pub fn lock_write(&self, repo_name: &str) -> MutexGuard<()> {
        Lock::lock(&self.write_locks, repo_name)
    }

    pub fn is_repo_syncing(&self, repo_name: &str) -> bool {
        let map = self.sync_locks.lock().unwrap();
        if let Some(atomic) = map.get(repo_name) {
            if atomic.load(Ordering::SeqCst) {
                true
            } else {
                false
            }
        } else {
            true
        }
    }
}

#[cfg(test)]
pub mod test {
    use crate::locks::Lock;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn try_lock() {
        let lock = Arc::new(Lock::new());
        {
            let lock = lock.clone();
            let handler = thread::spawn(move || lock.lock_sync("repo").is_some());
            assert_eq!(true, handler.join().unwrap());
        }
        {
            let _holder = lock.lock_sync("repo").unwrap();
            assert!(lock.lock_sync("repo").is_none());
            let lock = lock.clone();
            let handler = thread::spawn(move || lock.lock_sync("repo").is_some());
            assert_eq!(false, handler.join().unwrap());
        }
        assert!(lock.lock_sync("repo").is_some())
    }

    #[test]
    fn lock() {
        let lock = Lock::new();
        {
            let _guard = lock.lock_write("repo");
        }
        let _guard = lock.lock_write("repo");
    }
}
