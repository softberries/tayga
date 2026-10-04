TAYGA_ROOT := $(CURDIR)
DEMO_DIR := $(TAYGA_ROOT)/vendor/opentelemetry-demo
export TAYGA_ROOT
export DEMO_VERSION := 3.1.0
export OTEL_COLLECTOR_CONFIG_EXTRAS := $(TAYGA_ROOT)/deploy/otelcol-config-tayga.yml

COMPOSE := docker compose --project-directory $(DEMO_DIR) \
	-f $(DEMO_DIR)/compose.yaml \
	-f $(DEMO_DIR)/compose.full.yaml \
	-f $(DEMO_DIR)/compose.observability.yaml \
	-f $(TAYGA_ROOT)/deploy/compose.infra.yaml \
	-f $(TAYGA_ROOT)/deploy/compose.tayga.yaml
INFRA := docker compose -p tayga-it -f $(TAYGA_ROOT)/deploy/compose.infra.yaml

.PHONY: up up-extras ui-dev ui-e2e down ps logs infra-up infra-down it flags-reset flag verify-raw capture e2e

up:
	git submodule update --init
	$(COMPOSE) build tayga-migrate
	$(COMPOSE) up -d

# Grafana and Prometheus (profile "extras"), plus Grafana/Jaeger links in the app.
up-extras:
	$(COMPOSE) -f $(TAYGA_ROOT)/deploy/compose.extras.yaml --profile extras up -d

# The profile makes down also remove tayga-grafana and tayga-prometheus if running.
down:
	$(COMPOSE) --profile extras down

ui-dev:
	npm --prefix ui run dev

ui-e2e:
	npm --prefix ui run e2e

ps:
	$(COMPOSE) ps

logs:
	$(COMPOSE) logs -f $(SERVICE)

infra-up:
	$(INFRA) up -d --wait

infra-down:
	$(INFRA) down -v

it: infra-up
	TAYGA_IT_KAFKA=localhost:19092 TAYGA_IT_CLICKHOUSE=http://localhost:18123 \
		cargo test --workspace --exclude tayga-e2e -- --ignored --test-threads=1

flags-reset:
	cp $(DEMO_DIR)/src/flagd/demo.flagd.json $(TAYGA_ROOT)/deploy/flagd/demo.flagd.json

flag:
	cargo run -q -p tayga-devtools -- flag $(NAME) $(VARIANT)

verify-raw:
	cargo run -q -p tayga-devtools -- verify-raw

capture:
	cargo run -q -p tayga-devtools -- capture --out fixtures/$(NAME).pb.gz $(ARGS)

e2e: flags-reset
	cargo test -p tayga-e2e -- --ignored --test-threads=1 --nocapture
