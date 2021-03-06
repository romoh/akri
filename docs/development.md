# Development
This document will walk you through how to set up a local development environment, build Akri component containers, and test Akri using your newly built containers.

The document includes [naming guidelines](#naming-guidelines) to help as you extend Akri.

## Required Tools
To develop, you'll need:
- A Linux environment whether on amd64 or arm64v8
- Rust - version 1.49.0 which the build system uses can be installed using: 
    ```sh
    sudo curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain=1.49.0
    cargo version
    ```
- .NET - the ONVIF broker is written in .NET, which can be installed according to [.NET instructions](https://docs.microsoft.com/dotnet/core/install/linux-ubuntu)

In order to cross-build containers for both ARM and x64, several tools are leveraged:

- The [Cross](https://github.com/rust-embedded/cross) tool can be installed with this command: `cargo install cross`.
- `qemu` can be installed with:
  ```sh
  sudo apt-get install -y qemu qemu qemu-system-misc qemu-user-static qemu-user binfmt-support
  ```

  For `qemu` to be fully configured on Ubuntu 18.04, after running apt-get install, run these commands:
  ```sh
    sudo mkdir -p /lib/binfmt.d
    sudo sh -c 'echo :qemu-arm:M::\\x7fELF\\x01\\x01\\x01\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x02\\x00\\x28\\x00:\\xff\\xff\\xff\\xff\\xff\\xff\\xff\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\xfe\\xff\\xff\\xff:/usr/bin/qemu-arm-static:F > /lib/binfmt.d/qemu-arm-static.conf'
    sudo sh -c 'echo :qemu-aarch64:M::\\x7fELF\\x02\\x01\\x01\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x02\\x00\\xb7\\x00:\\xff\\xff\\xff\\xff\\xff\\xff\\xff\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\xfe\\xff\\xff\\xff:/usr/bin/qemu-aarch64-static:F > /lib/binfmt.d/qemu-aarch64-static.conf'
    sudo systemctl restart systemd-binfmt.service
  ```
## Build and Test

### Local builds and tests
1. Navigate to the repo's top folder (where this README is)
1. Install prerequisites
    ```sh
    ./build/setup.sh
    ```
1. Build Controller, Agent, and udev broker
    ```sh
    cargo build
    ```
1. Build ONVIF broker
    ```sh
    cd ./samples/brokers/onvif-video-broker
    dotnet build
    ```

There are unit tests for all of the Rust code.  To run all unit tests, simply navigate to the repo's top folder (where this README is) and type `cargo test`

To locally run the controller as part of a k8s cluster, follow these steps:

1.  Create or provide access to a valid cluster configuration by setting KUBECONFIG (can be done in the commandline) ...
    for the sake of this, the config is assumed to be in ~/test.cluster.config
1.  Build the repo with all default features by running `cargo build`
    > Note: By default, the Agent does not have embedded Discovery Handlers. To allow embedded Discovery Handlers in the
    > Agent, turn on the `agent-full` feature and the feature for each Discovery Handler you wish to embed -- Debug echo
    > is always included if `agent-full` is turned on. For example, to build an Agent with OPC UA, ONVIF, udev, and
    > debug echo Discovery Handlers: `cargo build --manifest-path agent/Cargo.toml --features "agent-full udev-feat
    > opcua-feat onvif-feat"`.
1.  Run the desired component 

    Run the **Controller** locally with info-level logging: `RUST_LOG=info KUBECONFIG=~/test.cluster.config
    ./target/debug/controller`

    Run the **Agent** locally with info-level logging: 
    ```sh
    sudo DEBUG_ECHO_INSTANCES_SHARED=true ENABLE_DEBUG_ECHO=1 RUST_LOG=info KUBECONFIG=~/test.cluster.config DISCOVERY_HANDLERS_DIRECTORY=~/tmp/akri AGENT_NODE_NAME=myNode HOST_CRICTL_PATH=/usr/bin/crictl HOST_RUNTIME_ENDPOINT=/var/run/dockershim.sock HOST_IMAGE_ENDPOINT=/var/run/dockershim.sock ./target/debug/agent
    ```
    > Note: The environment variables `HOST_CRICTL_PATH`, `HOST_RUNTIME_ENDPOINT`, and `HOST_IMAGE_ENDPOINT` are for
    > slot-reconciliation (making sure Pods that no longer exist are not still claiming Akri resources). The values of
    > these vary based on Kubernetes distribution. The above is for vanilla Kubernetes. For MicroK8s, use
    > `HOST_CRICTL_PATH=/usr/local/bin/crictl HOST_RUNTIME_ENDPOINT=/var/snap/microk8s/common/run/containerd.sock
    > HOST_IMAGE_ENDPOINT=/var/snap/microk8s/common/run/containerd.sock` and for K3s, use
    > `HOST_CRICTL_PATH=/usr/local/bin/crictl HOST_RUNTIME_ENDPOINT=/run/k3s/containerd/containerd.sock
    > HOST_IMAGE_ENDPOINT=/run/k3s/containerd/containerd.sock`.

    To run **Discovery Handlers** locally, simply navigate to the Discovery Handler under
    `akri/discovery-handler-modules/` and run using cargo run, setting where the Discovery Handler socket should be
    created in the `DISCOVERY_HANDLERS_DIRECTORY` variable. For example, to run the ONVIF Discovery Handler locally:
    ```sh
    cd akri/discovery-handler-modules/onvif-discovery-handler/
    RUST_LOG=info DISCOVERY_HANDLERS_DIRECTORY=~/tmp/akri AGENT_NODE_NAME=myNode cargo run
    ```
    To run the [debug echo Discovery Handler](#testing-with-debug-echo-discovery-handler), an environment variable,
    `DEBUG_ECHO_INSTANCES_SHARED`, must be set to specify whether it should register with the Agent as discovering
    shared or unshared devices. Run the debug echo Discovery Handler to discover mock unshared devices like so:
    ```sh
    cd akri/discovery-handler-modules/debug-echo-discovery-handler/
    RUST_LOG=info DEBUG_ECHO_INSTANCES_SHARED=false DISCOVERY_HANDLERS_DIRECTORY=~/tmp/akri AGENT_NODE_NAME=myNode cargo run
    ```

### To build containers
`Makefile` has been created to help with the more complicated task of building the Akri components and containers for the various supported platforms.

#### Establish a container repository
Containers for Akri are currently hosted in `ghcr.io/deislabs/akri` using the new [GitHub container registry](https://github.blog/2020-09-01-introducing-github-container-registry/). Any container repository can be used for private containers. If you want to enable GHCR, you can follow the [getting started guide](https://docs.github.com/en/free-pro-team@latest/packages/getting-started-with-github-container-registry).

To build containers, log into the desired repository:
```sh
CONTAINER_REPOSITORY=<repo>
sudo docker login $CONTAINER_REPOSITORY
```

#### Build intermediate containers
To ensure quick builds, we have created a number of intermediate containers that rarely change.

By default, `Makefile` will try to create containers with tag following this format: `<repo>/$USER/<component>:<label>` where
* `<component>` = rust-crossbuild | opencv-base
* `<repo>` = `devcaptest.azurecr.io`
  * `<repo>` can be overridden by setting `REGISTRY=<desired repo>`
* `$USER` = the user executing `Makefile` (could be `root` if using sudo)
  * `<repo>/$USER` can be overridden by setting `PREFIX=<desired container path>`
* `<label>` = the label is defined in [../build/intermediate-containers.mk](../build/intermediate-containers.mk)

##### Rust cross-build containers
These containers are used by the `cross` tool to crossbuild the Akri Rust code.  There is a container built for each supported platform and they contain any required dependencies for Akri components to build.  The dockerfile can be found here: build/containers/intermediate/Dockerfile.rust-crossbuild-*
  ```sh
  # To make all of the Rust cross-build containers:
  PREFIX=$CONTAINER_REPOSITORY make rust-crossbuild
  # To make specific platform(s):
  PREFIX=$CONTAINER_REPOSITORY BUILD_AMD64=1 BUILD_ARM32=1 BUILD_ARM64=1 make rust-crossbuild
  ```
After building the cross container(s), update [Cross.toml](../Cross.toml) to point to your intermediate container(s).

##### .NET OpenCV containers
These containers allow the ONVIF broker to be created without rebuilding OpenCV for .NET each time.  There is a container built for AMD64 and it is used to crossbuild to each supported platform.  The dockerfile can be found here: build/containers/intermediate/Dockerfile.opencvsharp-build.
  ```sh
  # To make all of the OpenCV base containers:
  PREFIX=$CONTAINER_REPOSITORY make opencv-base
  # To make specific platform(s):
  PREFIX=$CONTAINER_REPOSITORY BUILD_AMD64=1 BUILD_ARM32=1 BUILD_ARM64=1 make opencv-base
  ```

#### Build Akri component containers
By default, `Makefile` will try to create containers with tag following this format: `<repo>/$USER/<component>:<label>` where
* `<component>` = controller | agent | etc
* `<repo>` = `devcaptest.azurecr.io`
  * `<repo>` can be overridden by setting `REGISTRY=<desired repo>`
* `$USER` = the user executing `Makefile` (could be `root` if using sudo)
  * `<repo>/$USER` can be overridden by setting `PREFIX=<desired container path>`
* `<label>` = v$(cat version.txt)
  * `<label>` can be overridden by setting `LABEL_PREFIX=<desired label>`

**Note**: Before building these final component containers, make sure you have built any necessary [intermediate containers](./#build-intermediate-containers). In particular, to build any of the rust containers (the Controller, Agent, or udev broker), you must first [build the cross-build containers](./#rust-cross-build-containers).

```sh
# To make all of the Akri containers:
PREFIX=$CONTAINER_REPOSITORY make akri
# To make a specific component:
PREFIX=$CONTAINER_REPOSITORY make akri-controller
PREFIX=$CONTAINER_REPOSITORY make akri-udev
PREFIX=$CONTAINER_REPOSITORY make akri-onvif
PREFIX=$CONTAINER_REPOSITORY make akri-opcua-monitoring
PREFIX=$CONTAINER_REPOSITORY make akri-anomaly-detection
PREFIX=$CONTAINER_REPOSITORY make akri-streaming
PREFIX=$CONTAINER_REPOSITORY make akri-agent
# To make an Agent with embedded Discovery Handlers, turn on the `agent-full` feature along with the 
# feature for any Discovery Handlers that should be embedded.
PREFIX=$CONTAINER_REPOSITORY BUILD_SLIM_AGENT=0 AGENT_FEATURES="agent-full onvif-feat opcua-feat udev-feat" make akri-agent-full 

# To make a specific component on specific platform(s):
PREFIX=$CONTAINER_REPOSITORY BUILD_AMD64=1 BUILD_ARM32=1 BUILD_ARM64=1 make akri-streaming

# To make a specific component on specific platform(s) with a specific label:
PREFIX=$CONTAINER_REPOSITORY LABEL_PREFIX=latest BUILD_AMD64=1 BUILD_ARM32=1 BUILD_ARM64=1 make akri-streaming
```

**NOTE:** If your docker install requires you to use `sudo`, this will conflict with the `cross` command.  This flow has helped:
```sh
sudo -s
source /home/$SUDO_USER/.cargo/env

# run make commands that crossbuild the Rust

exit
```

#### More information about Akri build
For more detailed information about the Akri build infrastructure, review the [Akri build infrastructure document](./building.md)

## Install Akri with newly built containers
When installing Akri using helm, you can set the `imagePullSecrets`, `image.repository` and `image.tag` [Helm values](../deployment/helm/values.yaml) to point to your newly created containers. For example, to install Akri with with custom Controller and Agent containers, run the following, specifying the `image.tag` version to reflect [version.txt](../version.txt):
```bash
kubectl create secret docker-registry <your-secret-name> --docker-server=ghcr.io  --docker-username=<your-github-alias> --docker-password=<your-github-token>
helm repo add akri-helm-charts https://deislabs.github.io/akri/
helm install akri akri-helm-charts/akri-dev \
    --set imagePullSecrets[0].name="<your-secret-name>" \
    --set agent.image.repository="ghcr.io/<your-github-alias>/agent" \
    --set agent.image.tag="v<akri-version>-amd64" \
    --set controller.image.repository="ghcr.io/<your-github-alias>/controller" \
    --set controller.image.tag="v<akri-version>-amd64"
```

More information about the Akri Helm charts can be found in the [user guide](./user-guide.md#understanding-akri-helm-charts).

## Other useful Helm Commands
### Helm Package
If you make changes to anything in the [helm folder](../deployment/helm), you will probably need to create a new Helm chart for Akri. This can be done using the [`helm package`](https://helm.sh/docs/helm/helm_package/) command. To create a chart using the current state of the Helm templates and CRDs, run (from one level above the Akri directory) `helm package akri/deployment/helm/`. You will see a tgz file called `akri-<akri-version>.tgz` at the location where you ran the command. Now, install Akri using that chart:
```sh
helm install akri akri-<akri-version>.tgz \
    --set useLatestContainers=true
```
### Helm Template
When you install Akri using Helm, Helm creates the DaemonSet, Deployment, and Configuration yamls for you (using the values set in the install command) and applies them to the cluster. To inspect those yamls before installing Akri, you can use [`helm template`](https://helm.sh/docs/helm/helm_template/). 
For example, you will see the image in the Agent DaemonSet set to `image: "ghcr.io/<your-github-alias>/agent:v<akri-version>-amd64"` if you run the following:
```sh
helm template akri deployment/helm/ \
  --set imagePullSecrets[0].name="<your-secret-name>" \
  --set agent.image.repository="ghcr.io/<your-github-alias>/agent" \
  --set agent.image.tag="v<akri-version>-amd64"
```

### Helm Get Manifest
Run the following to inspect an already running Akri installation in order to see the currently applied yamls such as the Configuration CRD, Instance CRD, protocol Configurations, Agent DaemonSet, and Controller Deployment:
```sh
helm get manifest akri | less
```

### Helm Upgrade
To modify an Akri installation to reflect a new state, you can use [`helm upgrade`](https://helm.sh/docs/helm/helm_upgrade/). See the [Customizing an Akri Installation document](./customizing-akri-installation.md) for further explanation. 

## Testing with Debug Echo Discovery Handler
In order to kickstart using and debugging Akri, a debug echo Discovery Handler has been created. See its
[documentation](./debug-echo-configuration.md) to start using it.

## Naming Guidelines

One of the [two hard things](https://martinfowler.com/bliki/TwoHardThings.html) in Computer Science is naming things. It is proposed that Akri adopt naming guidelines to make developers' lives easier by providing consistency and reduce naming complexity.

Akri existed before naming guidelines were documented and may not employ the guidelines summarized here. However, it is hoped that developers will, at least, consider these guidelines when extending Akri.

### General Principles

+ Akri uses English
+ Akri is written principally in Rust, and Rust [naming](https://rust-lang.github.io/api-guidelines/naming.html) conventions are used
+ Types need not be included in names unless ambiguity would result
+ Shorter, simpler names are preferred

### Akri Discovery Handlers

Various Discovery Handlers have been developed: `debug_echo`, `onvif`, `opcua`, `udev`

Guidance:

+ `snake_case` names
+ (widely understood) initializations|acronyms are preferred

### Akri Brokers

Various Brokers have been developed: `onvif-video-broker`, `opcua-monitoring-broker`, `udev-video-broker`

Guidance:

+ Broker names should reflect Discovery Handler (Protocol) names and be suffixed `-broker`
+ Use Programming language-specific naming conventions when developing Brokers in non-Rust languages

> **NOTE** Even though the initialization of [ONVIF](https://en.wikipedia.org/wiki/ONVIF) includes "Video", the specification is broader than video and the broker name adds specificity by including the word (`onvif-video-broker`) in order to effectively describe its functionality.

### Kubernetes Resources

Various Kubernetes Resources have been developed:

+ CRDS: `Configurations`, `Instances`
+ Instances: `akri-agent-daemonset`, `akri-controller-deployment`, `akri-onvif`, `akri-opcua`, `akri-udev`

Guidance:

+ Kubernetes Convention is that resources (e.g. `DaemonSet`) and CRDs use (upper) CamelCase
+ Akri Convention is that Akri Kubernetes resources be prefixed `akri-`, e.g. `akri-agent-daemonset`
+ Names combining words should use hypens (`-`) to separate the words e.g. `akri-debug-echo`

> **NOTE** `akri-agent-daemonset` contradicts the general principle of not including types, if it had been named after these guidelines were drafted, it would be named `akri-agent`.
>
> Kubernetes' resources are strongly typed and the typing is evident through the CLI e.g. `kubectl get daemonsets/akri-agent-daemonset` and through a resource's `Kind` (e.g. `DaemonSet`). Including such types in the name is redundant.
