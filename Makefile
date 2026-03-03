.PHONY: build release test check fmt clippy lint clean docker-build docker-run help

VERSION := $(shell grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
IMAGE   := reposync
TAG     := $(VERSION)

## Build

build: ## Build in debug mode
	cargo build

release: ## Build in release mode
	cargo build --release

check: ## Run cargo check (fast compilation check)
	cargo check

## Quality

test: ## Run all tests
	cargo test

fmt: ## Format code
	cargo fmt

fmt-check: ## Check formatting without modifying files
	cargo fmt -- --check

clippy: ## Run clippy lints
	cargo clippy -- -D warnings

lint: fmt-check clippy ## Run all linters (format check + clippy)

## Docker

docker-build: ## Build Docker image
	docker build -f deployment/Dockerfile -t $(IMAGE):$(TAG) .

docker-build-latest: docker-build ## Build and tag as latest
	docker tag $(IMAGE):$(TAG) $(IMAGE):latest

## Cleanup

clean: ## Remove build artifacts
	cargo clean

## Help

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-20s\033[0m %s\n", $$1, $$2}'
