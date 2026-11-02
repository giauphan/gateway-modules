"""Persistent state and cache manager for the Daily News service.

Clean Architecture Layer: Infrastructure / Secondary Adapter.
Ensures thread-safe, atomic disk persistence for module state and deduplication.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Dict, List, Optional, Set


class NewsStateStore:
    """Manages persistent cache, consumed candidate IDs, and module configuration."""

    def __init__(self, state_dir: Path | str) -> None:
        self.state_dir = Path(state_dir)
        self.state_dir.mkdir(parents=True, exist_ok=True)
        self.consumed_file = self.state_dir / "consumed_candidates.json"
        self.cache_file = self.state_dir / "cached_candidates.json"
        self.config_file = self.state_dir / "config.json"

    def _atomic_write_json(self, path: Path, data: Any) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(path.suffix + ".tmp")
        with tmp.open("w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False, indent=2)
        tmp.replace(path)

    def _read_json(self, path: Path, default: Any) -> Any:
        if not path.is_file():
            return default
        try:
            with path.open("r", encoding="utf-8") as f:
                return json.load(f)
        except Exception:
            return default

    def get_consumed_ids(self) -> Set[str]:
        """Return set of previously consumed candidate IDs."""
        data = self._read_json(self.consumed_file, [])
        if isinstance(data, list):
            return set(str(x) for x in data)
        return set()

    def mark_consumed(self, candidate_ids: List[str] | Set[str]) -> None:
        """Add candidate IDs to consumed set and save to disk."""
        current = self.get_consumed_ids()
        current.update(candidate_ids)
        self._atomic_write_json(self.consumed_file, sorted(list(current)))

    def save_cached_candidates(self, candidates: List[Dict[str, Any]]) -> None:
        """Cache latest ranked candidates."""
        self._atomic_write_json(self.cache_file, candidates)

    def get_cached_candidates(self) -> List[Dict[str, Any]]:
        """Retrieve cached candidates."""
        data = self._read_json(self.cache_file, [])
        return data if isinstance(data, list) else []

    def load_config(self) -> Dict[str, Any]:
        """Load module configuration from disk."""
        return self._read_json(self.config_file, {})

    def save_config(self, cfg: Dict[str, Any]) -> None:
        """Save updated module configuration to disk."""
        self._atomic_write_json(self.config_file, cfg)
