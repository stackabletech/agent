//! Accessor methods for Kubernetes resources

use std::str::FromStr;

use kubelet::pod::Pod;
use strum::{Display, EnumString, EnumVariantNames};

/// Restart policy for all containers within the pod.
#[derive(Clone, Debug, Display, EnumString, EnumVariantNames, Eq, PartialEq)]
pub enum RestartPolicy {
    Always,
    OnFailure,
    Never,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        RestartPolicy::Always
    }
}

/// Returns the restart policy for all containers within the pod.
pub fn restart_policy(pod: &Pod) -> RestartPolicy {
    pod.as_kube_pod()
        .spec
        .as_ref()
        .and_then(|spec| spec.restart_policy.as_ref())
        .and_then(|restart_policy| RestartPolicy::from_str(restart_policy).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::provider::test::TestPod;
    use rstest::rstest;

    #[rstest]
    #[case::restart_policy_onfailure(
        "
            apiVersion: v1
            kind: Pod
            metadata:
              name: test
            spec:
              containers:
                - name: test-container
              restartPolicy: OnFailure
        ",
        RestartPolicy::OnFailure
    )]
    #[case::restart_policy_default(
        "
            apiVersion: v1
            kind: Pod
            metadata:
              name: test
            spec:
              containers:
                - name: test-container
        ",
        RestartPolicy::Always
    )]
    fn should_return_specified_restart_policy_or_default(
        #[case] pod: TestPod,
        #[case] expected_restart_policy: RestartPolicy,
    ) {
        assert_eq!(expected_restart_policy, restart_policy(&pod));
    }
}
