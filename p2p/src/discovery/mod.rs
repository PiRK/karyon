mod lookup;
mod refresh;

use std::sync::Arc;

use log::{error, info};
use rand::{rngs::OsRng, seq::SliceRandom};
use smol::lock::Mutex;

use karyons_core::{
    async_utils::{Backoff, TaskGroup, TaskResult},
    GlobalExecutor,
};

use karyons_net::{Conn, Endpoint};

use crate::{
    config::Config,
    connection::{ConnDirection, ConnQueue},
    connector::Connector,
    listener::Listener,
    monitor::Monitor,
    routing_table::{
        Entry, EntryStatusFlag, RoutingTable, CONNECTED_ENTRY, DISCONNECTED_ENTRY, PENDING_ENTRY,
        UNREACHABLE_ENTRY, UNSTABLE_ENTRY,
    },
    slots::ConnectionSlots,
    Error, PeerID, Result,
};

use lookup::LookupService;
use refresh::RefreshService;

pub type ArcDiscovery = Arc<Discovery>;

pub struct Discovery {
    /// Routing table
    table: Arc<Mutex<RoutingTable>>,

    /// Lookup Service
    lookup_service: Arc<LookupService>,

    /// Refresh Service
    refresh_service: Arc<RefreshService>,

    /// Connector
    connector: Arc<Connector>,
    /// Listener
    listener: Arc<Listener>,

    /// Connection queue
    conn_queue: Arc<ConnQueue>,

    /// Inbound slots.
    pub(crate) inbound_slots: Arc<ConnectionSlots>,
    /// Outbound slots.
    pub(crate) outbound_slots: Arc<ConnectionSlots>,

    /// Managing spawned tasks.
    task_group: TaskGroup<'static>,

    /// Holds the configuration for the P2P network.
    config: Arc<Config>,
}

impl Discovery {
    /// Creates a new Discovery
    pub fn new(
        peer_id: &PeerID,
        conn_queue: Arc<ConnQueue>,
        config: Arc<Config>,
        monitor: Arc<Monitor>,
        ex: GlobalExecutor,
    ) -> ArcDiscovery {
        let inbound_slots = Arc::new(ConnectionSlots::new(config.inbound_slots));
        let outbound_slots = Arc::new(ConnectionSlots::new(config.outbound_slots));

        let table_key = peer_id.0;
        let table = Arc::new(Mutex::new(RoutingTable::new(table_key)));

        let refresh_service =
            RefreshService::new(config.clone(), table.clone(), monitor.clone(), ex.clone());
        let lookup_service = LookupService::new(
            peer_id,
            table.clone(),
            config.clone(),
            monitor.clone(),
            ex.clone(),
        );

        let connector = Connector::new(
            config.max_connect_retries,
            outbound_slots.clone(),
            monitor.clone(),
            ex.clone(),
        );
        let listener = Listener::new(inbound_slots.clone(), monitor.clone(), ex.clone());

        Arc::new(Self {
            refresh_service: Arc::new(refresh_service),
            lookup_service: Arc::new(lookup_service),
            conn_queue,
            table,
            inbound_slots,
            outbound_slots,
            connector,
            listener,
            task_group: TaskGroup::new(ex),
            config,
        })
    }

    /// Start the Discovery
    pub async fn start(self: &Arc<Self>) -> Result<()> {
        // Check if the listen_endpoint is provided, and if so, start a listener.
        if let Some(endpoint) = &self.config.listen_endpoint {
            // Return an error if the discovery port is set to 0.
            if self.config.discovery_port == 0 {
                return Err(Error::Config(
                    "Please add a valid discovery port".to_string(),
                ));
            }

            let resolved_endpoint = self.start_listener(endpoint).await?;

            if endpoint.addr()? != resolved_endpoint.addr()? {
                info!("Resolved listen endpoint: {resolved_endpoint}");
                self.lookup_service
                    .set_listen_endpoint(&resolved_endpoint)
                    .await;
                self.refresh_service
                    .set_listen_endpoint(&resolved_endpoint)
                    .await;
            }
        }

        // Start the lookup service
        self.lookup_service.start().await?;
        // Start the refresh service
        self.refresh_service.start().await?;

        // Attempt to manually connect to peer endpoints provided in the Config.
        for endpoint in self.config.peer_endpoints.iter() {
            let _ = self.connect(endpoint, None).await;
        }

        // Start connect loop
        let selfc = self.clone();
        self.task_group
            .spawn(selfc.connect_loop(), |res| async move {
                if let TaskResult::Completed(Err(err)) = res {
                    error!("Connect loop stopped: {err}");
                }
            });

        Ok(())
    }

    /// Shuts down the discovery
    pub async fn shutdown(&self) {
        self.task_group.cancel().await;
        self.connector.shutdown().await;
        self.listener.shutdown().await;

        self.refresh_service.shutdown().await;
        self.lookup_service.shutdown().await;
    }

    /// Start a listener and on success, return the resolved endpoint.
    async fn start_listener(self: &Arc<Self>, endpoint: &Endpoint) -> Result<Endpoint> {
        let selfc = self.clone();
        let callback = |conn: Conn| async move {
            selfc.conn_queue.handle(conn, ConnDirection::Inbound).await;
            Ok(())
        };

        let resolved_endpoint = self.listener.start(endpoint.clone(), callback).await?;
        Ok(resolved_endpoint)
    }

    /// This method will attempt to connect to a peer in the routing table.
    /// If the routing table is empty, it will start the seeding process for
    /// finding new peers.
    ///
    /// This will perform a backoff to prevent getting stuck in the loop
    /// if the seeding process couldn't find any peers.
    async fn connect_loop(self: Arc<Self>) -> Result<()> {
        let backoff = Backoff::new(500, self.config.seeding_interval * 1000);
        loop {
            let random_entry = self.random_entry(PENDING_ENTRY).await;
            match random_entry {
                Some(entry) => {
                    backoff.reset();
                    let endpoint = Endpoint::Tcp(entry.addr, entry.port);
                    self.connect(&endpoint, Some(entry.key.into())).await;
                }
                None => {
                    backoff.sleep().await;
                    self.start_seeding().await;
                }
            }
        }
    }

    /// Connect to the given endpoint using the connector
    async fn connect(self: &Arc<Self>, endpoint: &Endpoint, pid: Option<PeerID>) {
        let selfc = self.clone();
        let pid_cloned = pid.clone();
        let cback = |conn: Conn| async move {
            selfc.conn_queue.handle(conn, ConnDirection::Outbound).await;
            if let Some(pid) = pid_cloned {
                selfc.update_entry(&pid, DISCONNECTED_ENTRY).await;
            }
            Ok(())
        };

        let res = self.connector.connect_with_cback(endpoint, cback).await;

        if let Some(pid) = &pid {
            match res {
                Ok(_) => {
                    self.update_entry(pid, CONNECTED_ENTRY).await;
                }
                Err(_) => {
                    self.update_entry(pid, UNREACHABLE_ENTRY).await;
                }
            }
        }
    }

    /// Starts seeding process.
    ///
    /// This method randomly selects a peer from the routing table and
    /// attempts to connect to that peer for the initial lookup. If the routing
    /// table doesn't have an available entry, it will connect to one of the
    /// provided bootstrap endpoints in the `Config` and initiate the lookup.
    async fn start_seeding(&self) {
        match self.random_entry(PENDING_ENTRY | CONNECTED_ENTRY).await {
            Some(entry) => {
                let endpoint = Endpoint::Tcp(entry.addr, entry.discovery_port);
                if let Err(err) = self.lookup_service.start_lookup(&endpoint).await {
                    self.update_entry(&entry.key.into(), UNSTABLE_ENTRY).await;
                    error!("Failed to do lookup: {endpoint}: {err}");
                }
            }
            None => {
                let peers = &self.config.bootstrap_peers;
                for endpoint in peers.choose_multiple(&mut OsRng, peers.len()) {
                    if let Err(err) = self.lookup_service.start_lookup(endpoint).await {
                        error!("Failed to do lookup: {endpoint}: {err}");
                    }
                }
            }
        }
    }

    /// Returns a random entry from routing table.
    async fn random_entry(&self, entry_flag: EntryStatusFlag) -> Option<Entry> {
        self.table.lock().await.random_entry(entry_flag).cloned()
    }

    /// Update the entry status  
    async fn update_entry(&self, pid: &PeerID, entry_flag: EntryStatusFlag) {
        let table = &mut self.table.lock().await;
        table.update_entry(&pid.0, entry_flag);
    }
}