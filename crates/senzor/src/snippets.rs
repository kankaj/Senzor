//! Text to run the open project elsewhere: a GitHub Actions step and a docker-compose service,
//! as the CI and Docker recipe in the documentation runs it.

use senzor_core::project::ProjectFile;

/// Which snippet to copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Snippet {
    GitHubActions,
    DockerCompose,
}

impl Snippet {
    /// What the copy button and the palette call it.
    pub fn label(self) -> &'static str {
        match self {
            Snippet::GitHubActions => "Copy as a GitHub Actions step",
            Snippet::DockerCompose => "Copy as a docker-compose service",
        }
    }
}

/// The snippet for the project saved as `file_name`.
pub fn text(snippet: Snippet, file_name: &str, file: &ProjectFile) -> String {
    let password_env = file.broker.password_env.as_deref();
    match snippet {
        Snippet::GitHubActions => {
            let mut text = String::from("- name: Run the simulated sensors beside the tests\n");
            if let Some(variable) = password_env {
                text.push_str(&format!(
                    "  env:\n    {variable}: ${{{{ secrets.{variable} }}}}\n"
                ));
            }
            text.push_str(&format!(
                "  run: |\n    \
                 senzor-cli validate {file_name}\n    \
                 senzor-cli run {file_name} --duration 10m &\n    \
                 SENZOR=$!\n    \
                 ./run-integration-tests.sh  # your tests\n    \
                 kill \"$SENZOR\" 2>/dev/null || true\n    \
                 wait \"$SENZOR\"\n"
            ));
            text
        }
        Snippet::DockerCompose => {
            let mut text = String::from(
                "services:\n  senzor:\n    \
                 # An image whose entry point is senzor-cli; see \"Senzor in a container\" in the docs.\n    \
                 image: senzor-cli\n",
            );
            if is_local(&file.broker.host) {
                text.push_str(
                    "    # The project's broker is localhost, which is the container itself: \
                     use a host the container can reach, such as a compose service's name.\n",
                );
            }
            text.push_str(&format!(
                "    command: [\"run\", \"/project/{file_name}\"]\n    \
                 volumes:\n      \
                 - ./{file_name}:/project/{file_name}:ro\n"
            ));
            if let Some(variable) = password_env {
                text.push_str(&format!(
                    "    environment:\n      {variable}: ${{{variable}}}\n"
                ));
            }
            text
        }
    }
}

/// Whether `host` names the machine itself, which inside a container is the container.
fn is_local(host: &str) -> bool {
    host == "localhost" || host.starts_with("127.") || host == "::1"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(broker: &str) -> ProjectFile {
        ProjectFile::parse(&format!(
            "[broker]\n{broker}\n\n[[devices]]\nid = \"d\"\ntemplate = \"co2\"\n"
        ))
        .unwrap()
    }

    #[test]
    fn a_github_actions_step_validates_then_runs_beside_the_tests() {
        let text = text(
            Snippet::GitHubActions,
            "plant.toml",
            &project("host = \"mqtt\""),
        );
        assert_eq!(
            text,
            "\
- name: Run the simulated sensors beside the tests
  run: |
    senzor-cli validate plant.toml
    senzor-cli run plant.toml --duration 10m &
    SENZOR=$!
    ./run-integration-tests.sh  # your tests
    kill \"$SENZOR\" 2>/dev/null || true
    wait \"$SENZOR\"
"
        );
    }

    #[test]
    fn a_password_variable_is_passed_from_a_secret() {
        let text = text(
            Snippet::GitHubActions,
            "plant.toml",
            &project("host = \"mqtt\"\nusername = \"ci\"\npassword_env = \"MQTT_PASSWORD\""),
        );
        assert!(
            text.contains("  env:\n    MQTT_PASSWORD: ${{ secrets.MQTT_PASSWORD }}\n"),
            "{text}"
        );
        let text = text_compose("host = \"mqtt\"\nusername = \"ci\"\npassword_env = \"PW\"");
        assert!(
            text.contains("    environment:\n      PW: ${PW}\n"),
            "{text}"
        );
    }

    fn text_compose(broker: &str) -> String {
        text(Snippet::DockerCompose, "plant.toml", &project(broker))
    }

    #[test]
    fn a_docker_compose_service_mounts_the_project_and_runs_it() {
        assert_eq!(
            text_compose("host = \"mqtt\""),
            "\
services:
  senzor:
    # An image whose entry point is senzor-cli; see \"Senzor in a container\" in the docs.
    image: senzor-cli
    command: [\"run\", \"/project/plant.toml\"]
    volumes:
      - ./plant.toml:/project/plant.toml:ro
"
        );
    }

    #[test]
    fn a_local_broker_is_flagged_for_a_container() {
        let text = text_compose("host = \"localhost\"");
        assert!(
            text.contains("# The project's broker is localhost, which is the container itself"),
            "{text}"
        );
    }
}
