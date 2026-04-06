DOCKER ?= podman
LINT_IMAGE=ghcr.io/igorshubovych/markdownlint-cli:v0.44.0

.PHONY: lint

lint:
	$(DOCKER) run --rm -v "$(PWD):/data:Z" -w /data $(LINT_IMAGE) --fix "**/*.md"