"""
Inward and Outward Port protocols for Meta Business Suite.
Clean Architecture Layer: Application Ports (Protocols for DIP decoupling).
"""

from typing import Any, Dict, List, Optional, Protocol, runtime_checkable

from .domain import PublishSpec, VideoUploadSession


@runtime_checkable
class ConfigStoragePort(Protocol):
    """Outward port for configuration loading and persistence."""

    def load_config(self) -> Dict[str, Any]:
        """Loads active module configuration."""
        ...

    def save_config(self, updates: Dict[str, Any]) -> Dict[str, Any]:
        """Persists updates to module configuration."""
        ...


@runtime_checkable
class FacebookTransportPort(Protocol):
    """Outward port for abstracting HTTP and rupload transport."""

    def post_form(
        self,
        url: str,
        form_data: Dict[str, Any],
        headers: Dict[str, str],
        timeout: int = 30,
    ) -> Dict[str, Any]:
        """Executes form-encoded POST request against Meta endpoints."""
        ...

    def post_binary(
        self,
        url: str,
        data: bytes,
        headers: Dict[str, str],
        timeout: int = 60,
    ) -> Dict[str, Any]:
        """Streams binary octet payload to Meta rupload endpoints."""
        ...


@runtime_checkable
class FacebookUploadUseCasePort(Protocol):
    """Inward port for video upload and publishing orchestration."""

    def upload_and_publish(
        self,
        video_bytes: bytes,
        file_name: str,
        title: str,
        description: str,
        target_id: Optional[str] = None,
        scheduled_publish_time: Optional[int] = None,
        enable_auto_translate: Optional[bool] = None,
        translated_languages: Optional[List[str]] = None,
    ) -> Dict[str, Any]:
        """Orchestrates end-to-end video upload and Reel publishing."""
        ...
