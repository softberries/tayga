# Tayga's load-generator override: the demo's locustfile minus the `ask_agent` task.
# The `agent` service lives in the demo's optional compose.agent.yaml layer, which Tayga
# does not run, so every ask_agent call fails and adds noise. The vendored file stays
# untouched; this module imports it and filters the task list Locust built for it.
from locustfile import *  # noqa: F401,F403  (re-exports the demo's User classes)
from locustfile import WebsiteUser

WebsiteUser.tasks = [t for t in WebsiteUser.tasks if getattr(t, "__name__", "") != "ask_agent"]
