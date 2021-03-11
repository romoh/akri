use super::crictl_containers;
use akri_shared::{
    akri::instance::{Instance, NodeName, Slot},
    k8s::{self, KubeInterface},
};
use async_trait::async_trait;
use futures::StreamExt;
use k8s_openapi::api::core::v1::PodStatus;
use kube::api::{Api, Informer, WatchEvent};
use mockall::automock;
use mockall::predicate::*;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::process::Command;

type SlotQueryResult = Result<HashSet<String>, Box<dyn std::error::Error + Send + Sync + 'static>>;

#[automock]
#[async_trait]
pub trait SlotQuery {
    async fn get_node_slots(&self) -> SlotQueryResult;
}

/// Discovers which of an instance's usage slots are actively used by containers on this node
pub struct CriCtlSlotQuery {
    pub crictl_path: String,
    pub runtime_endpoint: String,
    pub image_endpoint: String,
}

#[async_trait]
impl SlotQuery for CriCtlSlotQuery {
    /// Calls crictl to query container runtime in search of active containers and extracts their usage slots.
    async fn get_node_slots(&self) -> SlotQueryResult {
        match Command::new(&self.crictl_path)
            .args(&[
                "--runtime-endpoint",
                &self.runtime_endpoint,
                "--image-endpoint",
                &self.image_endpoint,
                "ps",
                "-v",
                "--output",
                "json",
            ])
            .output()
            .await
        {
            Ok(output) => {
                if output.status.success() {
                    trace!("get_node_slots - crictl called successfully");
                    let output_string = String::from_utf8_lossy(&output.stdout);
                    Ok(crictl_containers::get_container_slot_usage(&output_string))
                } else {
                    let output_string = String::from_utf8_lossy(&output.stderr);
                    Err(None.ok_or(format!(
                        "get_node_slots - Failed to call crictl: {:?}",
                        output_string
                    ))?)
                }
            }
            Err(e) => {
                trace!("get_node_slots - Command failed to call crictl: {:?}", e);
                Err(e.into())
            }
        }
    }
}

// Name of the pod
type Pod = String;

pub struct DevicePluginReconciler {
    pub slot_pod_map: Arc<Mutex<HashMap<Slot, Pod>>>,
}

/// Makes sure Instance's `device_usage` accurately reflects actual usage.
impl DevicePluginReconciler {
    pub fn add_or_update_slot(&self, slot: Slot) {
        // TODO: Add validation
        // Adds a new slot, pod info is unknown at this point
        self.slot_pod_map
            .lock()
            .unwrap()
            .insert(slot, String::new());
    }

    pub fn remove_slot(&self, slot: Slot) {
        self.slot_pod_map.lock().unwrap().remove(&slot);
    }

    pub async fn reconcile(
        &self,
        node_name: &str,
        slot_query: &impl SlotQuery,
        kube_interface: &impl KubeInterface,
    ) {
        // First, check if any of the existing pods running on this node that has a container
        // that is still not ready, this way we avoid incorrectly cleaning up slots during
        // container bring-up. When the container is ready, Informer will inform us that
        // the pod was modified and then reconcile will be called again
        let pods = match kube_interface
            .find_pods_with_field(&format!("{}={}", "spec.nodeName", &node_name,))
            .await
        {
            Ok(pods) => {
                trace!("reconcile - found {} pods on this node", pods.items.len());
                pods
            }
            Err(e) => {
                trace!("reconcile - error finding pending pods: {}", e);
                return;
            }
        };

        let any_unready_pods = pods.items.iter().any(|pod| {
            pod.status
                .as_ref()
                .unwrap_or(&PodStatus::default())
                .conditions
                .as_ref()
                .unwrap_or(&Vec::new())
                .iter()
                .any(|condition| condition.type_ == "ContainersReady" && condition.status != "True")
        });

        if any_unready_pods {
            info!("reconcile - pods with unready containers exist on this node, we can't reconcile slots yet");
            return;
        }

        // Use crictl to check which pods of the current node are currently assigned slots
        let node_slot_usage = match slot_query.get_node_slots().await {
            Ok(usage) => usage,
            Err(e) => {
                warn!("reconcile - get_node_slots failed: {:?}", e);
                // If an error occurs in the crictl call, return early
                // to avoid treating this error like crictl found no
                // active containers. Currently, reconcile is a best
                // effort approach.
                return;
            }
        };
        trace!(
            "reconcile - slots currently in use on this node: {:?}",
            node_slot_usage
        );

        // Get slot allocation as known to the kubernetes infrastructure
        let instances = match kube_interface.get_instances().await {
            Ok(instances) => instances,
            Err(e) => {
                trace!("reconcile - failed to get instances: {:?}", e);
                return;
            }
        };

        for instance in instances {
            // Check Instance against list of slots that are being used by this node's
            // current pods.  If we find any missing, we should update the Instance for
            // the actual slot usage.
            let slots_missing_this_node_name = instance
                .spec
                .device_usage
                .iter()
                .filter_map(|(slot, node)| {
                    if node != node_name && node_slot_usage.contains(slot) {
                        // We need to add node_name to this slot IF
                        //     the slot is not labeled with node_name AND
                        //     there is a container using that slot on this node
                        trace!(
                            "reconcile - slot {} assigned to node {}, instead of {}",
                            slot,
                            node,
                            node_name
                        );
                        Some(slot.to_string())
                    } else {
                        trace!("reconcile - slot {} assigned to node {}", slot, node);
                        None
                    }
                })
                .collect::<HashSet<Slot>>();

            // Check Instance to find slots that are registered to this node, but
            // there is no actual pod using the slot. We should update the Instance
            // to clear the false usage.
            let slots_to_clean = instance
                .spec
                .device_usage
                .iter()
                .filter_map(|(slot, node)| {
                    if node == node_name && !node_slot_usage.contains(slot) {
                        // We need to clean this slot IF
                        //     this slot is handled by this node AND
                        //     there are no containers using that slot on this node
                        Some(slot.to_string())
                    } else {
                        None
                    }
                })
                .collect::<HashSet<Slot>>();

            if !slots_to_clean.is_empty() || !slots_missing_this_node_name.is_empty() {
                debug!(
                    "reconcile - update Instance slots_to_clean: {:?}  slots_missing_this_node_name: {:?}",
                    slots_to_clean,
                    slots_missing_this_node_name
                );

                let modified_device_usage = instance
                    .spec
                    .device_usage
                    .iter()
                    .map(|(slot, node)| {
                        (
                            slot.to_string(),
                            if slots_missing_this_node_name.contains(slot) {
                                // Set this to node_name because there have been
                                // cases where a Pod is running (which corresponds
                                // to an Allocate call, but the Instance slot is empty.
                                node_name.into()
                            } else if slots_to_clean.contains(slot) {
                                // Set this to empty string because there is no
                                // Deallocate message from kubelet for us to know
                                // when a slot is no longer in use
                                "".into()
                            } else {
                                // This slot remains unchanged.
                                node.into()
                            },
                        )
                    })
                    .collect::<HashMap<Slot, NodeName>>();

                let modified_instance = Instance {
                    configuration_name: instance.spec.configuration_name.clone(),
                    metadata: instance.spec.metadata.clone(),
                    rbac: instance.spec.rbac.clone(),
                    shared: instance.spec.shared,
                    device_usage: modified_device_usage,
                    nodes: instance.spec.nodes.clone(),
                };
                info!("reconcile - update Instance from: {:?}", &instance.spec);
                info!("reconcile - update Instance   to: {:?}", &modified_instance);
                match kube_interface
                    .update_instance(
                        &modified_instance,
                        &instance.metadata.name,
                        &instance.metadata.namespace.unwrap(),
                    )
                    .await
                {
                    Ok(()) => {}
                    Err(e) => {
                        // If update fails, let the next iteration update the Instance.  We
                        // may want to revisit this decision and add some retry logic
                        // here.
                        error!("reconcile - update Instance failed: {:?}", e);
                    }
                }
            }
        }
        trace!("reconcile - thread iteration end");
    }
}

/// This watches pod to make sure that all Instances' device_usage (slots)
/// accurately reflects the actual usage.
///
/// The Kubernetes Device-Plugin implementation has no notifications for
/// when a Pod disappears (which should, in turn, free up a slot).  Because
/// of this, if a Pod disappears, there will be a slot that Akri (and the
/// Kubernetes scheduler) falsely thinks is in use.
///
/// To work around this, we have done 2 things:
///   1. Each of Agent's device plugins add slot information to the Annotations
///      section of the Allocate response.
///   2. watch_pods will invalidate and synchronize the slot allocation between each node and the API Server.
///      For each pod modified/deleted events, crictl is called to query the container runtime on this node in search of active Containers that have our slot
///      Annotations and comparing that to device usage (slots) allocated on the API Server.
///      This function will make sure that our Instance device_usage accurately reflects the actual usage.
///
/// It has rarely been seen, perhaps due to connectivity issues, that active
/// Containers with our Annotation are no longer in our Instance.  This is a bug that
/// we are aware of, but haven't found yet.  To address this, until a fix is found,
/// we will also make sure that any Container that exists with our Annotation will
/// be shown in our Instance device_usage.
pub async fn watch_pods() -> anyhow::Result<()> {
    let node_name = std::env::var("AGENT_NODE_NAME").unwrap();
    let kube_interface = k8s::create_kube_interface();
    let api = Api::v1Pod(kube_interface.get_kube_client());

    let informer = Informer::new(api.clone()).init().await?;

    // Create the crictl slot query to be used when a pod event is received
    let crictl_path = std::env::var("HOST_CRICTL_PATH").unwrap();
    let runtime_endpoint = std::env::var("HOST_RUNTIME_ENDPOINT").unwrap();
    let image_endpoint = std::env::var("HOST_IMAGE_ENDPOINT").unwrap();
    let slot_query = CriCtlSlotQuery {
        crictl_path,
        runtime_endpoint,
        image_endpoint,
    };

    let reconciler = DevicePluginReconciler {
        slot_pod_map: Arc::new(std::sync::Mutex::new(HashMap::new())),
    };

    loop {
        // starts a pod watch and returns a stream
        let mut pods = informer.poll().await?.boxed();

        while let Some(event) = pods.next().await {
            match event? {
                WatchEvent::Modified(pod) | WatchEvent::Deleted(pod) => {
                    // Filter out pods that do not belong to this node
                    if pod.spec.node_name.is_none()
                        || (pod.spec.node_name.is_some()
                            && !&pod.spec.node_name.unwrap().eq(&node_name))
                    {
                        break;
                    }

                    // if pod.status.is_some() && !pod.status.unwrap().container_statuses.started {
                    //     trace!("pod watch - pod {} has a container that is not ready")

                    // }

                    // Since some pod changes are happening, this is a good time to re-evaluate the allocated slots
                    let containers = pod
                        .spec
                        .containers
                        .into_iter()
                        .map(|c| c.name)
                        .collect::<Vec<_>>();
                    trace!(
                        "reconcile - pod event for {}, (containers={:?})",
                        pod.metadata.name,
                        containers
                    );

                    reconciler
                        .reconcile(&node_name, &slot_query, &kube_interface)
                        .await;
                }
                WatchEvent::Deleted(pod) => {
                    // if reconciler.slot_pod_map.contains(pod) {
                    //     reconciler.deallocate(pod);
                    // }
                }
                WatchEvent::Added(_) => {} // No need to handle adds, this will be handled by DevicePluginService's allocate
                WatchEvent::Error(e) => {
                    trace!("reconcile - error event: {:?}", e);
                }
            }
        }
    }
}

#[cfg(test)]
mod reconcile_tests {
    use super::*;
    use akri_shared::{akri::instance::KubeAkriInstanceList, k8s::MockKubeInterface, os::file};
    use k8s_openapi::api::core::v1::PodSpec;
    use kube::api::{Object, ObjectList};

    fn configure_get_node_slots(mock: &mut MockSlotQuery, result: HashSet<String>, error: bool) {
        mock.expect_get_node_slots().times(1).returning(move || {
            if !error {
                Ok(result.clone())
            } else {
                Err(None.ok_or("failure")?)
            }
        });
    }

    fn configure_get_instances(mock: &mut MockKubeInterface, result_file: &'static str) {
        mock.expect_get_instances().times(1).returning(move || {
            let instance_list_json = file::read_file_to_string(result_file);
            let instance_list: KubeAkriInstanceList =
                serde_json::from_str(&instance_list_json).unwrap();
            Ok(instance_list)
        });
    }

    fn configure_find_pods_with_field(
        mock: &mut MockKubeInterface,
        selector: &'static str,
        result_file: &'static str,
    ) {
        mock.expect_find_pods_with_field()
            .times(1)
            .withf(move |s| s == selector)
            .returning(move |_| {
                let pods_json = file::read_file_to_string(result_file);
                let pods: ObjectList<Object<PodSpec, PodStatus>> =
                    serde_json::from_str(&pods_json).unwrap();
                Ok(pods)
            });
    }

    struct NodeSlots {
        node_slots: HashSet<Slot>,
        node_slots_error: bool,
    }

    struct UpdateInstance {
        expected_slot_1_node: &'static str,
        expected_slot_5_node: &'static str,
    }

    async fn configure_scenario(
        node_slots: NodeSlots,
        instances_result_file: &'static str,
        update_instance: Option<UpdateInstance>,
    ) {
        let mut slot_query = MockSlotQuery::new();

        // slot_query to identify one slot used by this node
        configure_get_node_slots(
            &mut slot_query,
            node_slots.node_slots,
            node_slots.node_slots_error,
        );

        let mut kube_interface = MockKubeInterface::new();
        if !node_slots.node_slots_error {
            // kube_interface to find Instance with node-a using slots:
            //    config-a-359973-1 & config-a-359973-3
            configure_get_instances(&mut kube_interface, instances_result_file);

            // kube_interface to find no pods with unready containers
            configure_find_pods_with_field(
                &mut kube_interface,
                "spec.nodeName=node-a",
                "../test/json/running-pod-list-for-config-a-shared.json",
            );

            if let Some(update_instance_) = update_instance {
                trace!(
                    "expect_update_instance - slot1: {}, slot5: {}",
                    update_instance_.expected_slot_1_node,
                    update_instance_.expected_slot_5_node
                );
                // kube_interface to update Instance
                kube_interface
                    .expect_update_instance()
                    .times(1)
                    .withf(move |instance, name, namespace| {
                        name == "config-a-359973"
                            && namespace == "config-a-namespace"
                            && instance.nodes.len() == 3
                            && instance.nodes.contains(&"node-a".to_string())
                            && instance.nodes.contains(&"node-b".to_string())
                            && instance.nodes.contains(&"node-c".to_string())
                            && instance.device_usage["config-a-359973-0"] == "node-b"
                            && instance.device_usage["config-a-359973-1"]
                                == update_instance_.expected_slot_1_node
                            && instance.device_usage["config-a-359973-2"] == "node-b"
                            && instance.device_usage["config-a-359973-3"] == "node-a"
                            && instance.device_usage["config-a-359973-4"] == "node-c"
                            && instance.device_usage["config-a-359973-5"]
                                == update_instance_.expected_slot_5_node
                    })
                    .returning(move |_, _, _| Ok(()));
            }
        }

        reconcile("node-a", &slot_query, &kube_interface).await;
    }

    #[tokio::test]
    async fn test_reconcile_no_slots_to_reconcile() {
        let _ = env_logger::builder().is_test(true).try_init();

        configure_scenario(
            NodeSlots {
                node_slots: HashSet::new(),
                node_slots_error: false,
            },
            "../test/json/shared-instance-list.json",
            None,
        )
        .await;
    }

    #[tokio::test]
    async fn test_reconcile_get_slots_error() {
        let _ = env_logger::builder().is_test(true).try_init();

        configure_scenario(
            NodeSlots {
                node_slots: HashSet::new(),
                node_slots_error: true,
            },
            "",
            None,
        )
        .await;
    }

    #[tokio::test]
    async fn test_reconcile_slots_to_add() {
        let _ = env_logger::builder().is_test(true).try_init();

        let mut node_slots = HashSet::new();
        node_slots.insert("config-a-359973-3".to_string());
        node_slots.insert("config-a-359973-5".to_string());
        configure_scenario(
            // slot_query to identify one slot used by this node
            NodeSlots {
                node_slots,
                node_slots_error: false,
            },
            // kube_interface to find Instance with node-a using slots:
            //    config-a-359973-1 & config-a-359973-3
            "../test/json/shared-instance-list-slots.json",
            Some(UpdateInstance {
                expected_slot_1_node: "",
                expected_slot_5_node: "node-a",
            }),
        )
        .await;
    }

    #[tokio::test]
    async fn test_reconcile_slots_to_delete() {
        let _ = env_logger::builder().is_test(true).try_init();

        let mut node_slots = HashSet::new();
        node_slots.insert("config-a-359973-3".to_string());
        configure_scenario(
            // slot_query to identify one slot used by this node
            NodeSlots {
                node_slots: node_slots.clone(),
                node_slots_error: false,
            },
            // kube_interface to find Instance with node-a using slots:
            //    config-a-359973-1 & config-a-359973-3
            "../test/json/shared-instance-list-slots.json",
            Some(UpdateInstance {
                expected_slot_1_node: "",
                expected_slot_5_node: "",
            }),
        )
        .await;

        configure_scenario(
            // slot_query to identify one slot used by this node
            NodeSlots {
                node_slots: node_slots.clone(),
                node_slots_error: false,
            },
            // kube_interface to find Instance with node-a using slots:
            //    config-a-359973-1 & config-a-359973-3
            "../test/json/shared-instance-list-slots.json",
            Some(UpdateInstance {
                expected_slot_1_node: "",
                expected_slot_5_node: "",
            }),
        )
        .await;
    }

    #[tokio::test]
    async fn test_reconcile_slots_to_delete_and_add() {
        let _ = env_logger::builder().is_test(true).try_init();

        let mut node_slots = HashSet::new();
        node_slots.insert("config-a-359973-3".to_string());
        configure_scenario(
            // slot_query to identify one slot used by this node
            NodeSlots {
                node_slots,
                node_slots_error: false,
            },
            // kube_interface to find Instance with node-a using slots:
            //    config-a-359973-1 & config-a-359973-3
            "../test/json/shared-instance-list-slots.json",
            Some(UpdateInstance {
                expected_slot_1_node: "",
                expected_slot_5_node: "",
            }),
        )
        .await;

        let mut node_slots_added = HashSet::new();
        node_slots_added.insert("config-a-359973-3".to_string());
        node_slots_added.insert("config-a-359973-5".to_string());
        configure_scenario(
            // slot_query to identify one slot used by this node
            NodeSlots {
                node_slots: node_slots_added,
                node_slots_error: false,
            },
            // kube_interface to find Instance with node-a using slots:
            //    config-a-359973-1 & config-a-359973-3
            "../test/json/shared-instance-list-slots.json",
            Some(UpdateInstance {
                expected_slot_1_node: "",
                expected_slot_5_node: "node-a",
            }),
        )
        .await;
    }

    #[tokio::test]
    async fn test_reconcile_slots_to_delete_only_temporarily() {
        let _ = env_logger::builder().is_test(true).try_init();
        let mut node_slots = HashSet::new();
        node_slots.insert("config-a-359973-3".to_string());
        configure_scenario(
            // slot_query to identify one slot used by this node
            NodeSlots {
                node_slots,
                node_slots_error: false,
            },
            // kube_interface to find Instance with node-a using slots:
            //    config-a-359973-1 & config-a-359973-3
            "../test/json/shared-instance-list-slots.json",
            Some(UpdateInstance {
                expected_slot_1_node: "",
                expected_slot_5_node: "",
            }),
        )
        .await;

        let mut node_slots_added = HashSet::new();
        node_slots_added.insert("config-a-359973-1".to_string());
        node_slots_added.insert("config-a-359973-3".to_string());
        configure_scenario(
            // slot_query to identify two slots used by this node
            NodeSlots {
                node_slots: node_slots_added,
                node_slots_error: false,
            },
            // kube_interface to find Instance with node-a using slots:
            //    config-a-359973-1 & config-a-359973-3
            "../test/json/shared-instance-list-slots.json",
            None,
        )
        .await;
    }
}
