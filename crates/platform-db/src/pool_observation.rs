//! One observation owner per constructed pool; registry references are weak.
use crate::connection::Purpose;
use platform::{
    db::Backend,
    observability::{DatabaseBackend, DatabasePurpose, PoolObserver, PoolSample, metrics},
};
use std::sync::Arc;

struct DriverObservation<DB: sqlx::Database> {
    driver: sqlx::Pool<DB>,
    backend: DatabaseBackend,
    purpose: DatabasePurpose,
}
impl<DB: sqlx::Database> std::fmt::Debug for DriverObservation<DB> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriverObservation")
            .field("backend", &self.backend)
            .field("purpose", &self.purpose)
            .finish_non_exhaustive()
    }
}
impl<DB: sqlx::Database> PoolObserver for DriverObservation<DB> {
    fn backend(&self) -> DatabaseBackend {
        self.backend
    }
    fn purpose(&self) -> DatabasePurpose {
        self.purpose
    }
    fn sample(&self) -> PoolSample {
        // sqlx exposes independent gauges rather than an atomic snapshot. Clamp
        // idle if a checkout/return races the reads so in_use cannot underflow.
        let open = u64::from(self.driver.size());
        let idle = (self.driver.num_idle() as u64).min(open);
        PoolSample {
            pools: 1,
            open,
            idle,
            in_use: open - idle,
            max: u64::from(self.driver.options().get_max_connections()),
        }
    }
}

pub(crate) fn register<DB: sqlx::Database>(
    driver: &sqlx::Pool<DB>,
    backend: Backend,
    purpose: Purpose,
) -> Arc<dyn PoolObserver> {
    let observer: Arc<dyn PoolObserver> = Arc::new(DriverObservation {
        driver: driver.clone(),
        backend: backend.into(),
        purpose: match purpose {
            Purpose::App => DatabasePurpose::App,
            Purpose::Worker => DatabasePurpose::Worker,
            Purpose::Migrator => DatabasePurpose::Migrator,
        },
    });
    metrics().pools.register(&observer);
    observer
}
