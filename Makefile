SHELL := /usr/bin/env bash
.DEFAULT_GOAL := help

.PHONY: help setup build test check run verify security clean docker-build docker-run

help:
	@printf '%s\n' \
		'Option Workstation commands:' \
		'  make setup        Install locked frontend dependencies' \
		'  make build        Build frontend and release Rust server' \
		'  make test         Run Rust and legacy Python tests' \
		'  make check        Run formatting, linting, tests, and frontend build' \
		'  make run          Build and start the local workstation' \
		'  make verify       Verify code and a running server when available' \
		'  make security     Run publication and secret checks' \
		'  make docker-build Build the local container image' \
		'  make docker-run   Start with Docker Compose' \
		'  make clean        Remove generated build artifacts'

setup:
	cd frontend && npm ci

build: setup
	cd frontend && npm run build
	cargo build --locked --release --manifest-path rust-backend/Cargo.toml

test:
	cargo test --locked --manifest-path rust-backend/Cargo.toml
	python3 -m pytest -q

check:
	./scripts/verify.sh

run:
	./scripts/start.sh

verify:
	./scripts/verify.sh

security:
	./scripts/publication-check.sh

docker-build:
	docker build --tag option-workstation:local .

docker-run:
	docker compose up --build

clean:
	rm -rf frontend/dist frontend/node_modules rust-backend/target artifacts .pytest_cache
