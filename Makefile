.PHONY: build install start stop restart logs status clean

build:
	cargo build --release

install: build
	mkdir -p $(HOME)/.local/bin
	cp target/release/snap $(HOME)/.local/bin/snap
ifeq ($(shell uname -s),Darwin)
	mkdir -p $(HOME)/Library/LaunchAgents
	sed 's|%SNAP_BIN%|$(HOME)/.local/bin/snap|g; s|%HOME_PATH%|$(HOME)|g' snap.plist \
	  > $(HOME)/Library/LaunchAgents/com.adjective.snap.plist
	launchctl unload $(HOME)/Library/LaunchAgents/com.adjective.snap.plist 2>/dev/null || true
	launchctl load -w $(HOME)/Library/LaunchAgents/com.adjective.snap.plist
else
	mkdir -p $(HOME)/.config/systemd/user
	sed 's|%SNAP_BIN%|$(HOME)/.local/bin/snap|g' snap.service > $(HOME)/.config/systemd/user/snap.service
	systemctl --user daemon-reload
	systemctl --user enable --now snap.service
endif

start:
	$(HOME)/.local/bin/snap start

stop:
	$(HOME)/.local/bin/snap stop

status:
	$(HOME)/.local/bin/snap status

logs:
	tail -f $(HOME)/.snap/snap.log

clean:
	cargo clean
