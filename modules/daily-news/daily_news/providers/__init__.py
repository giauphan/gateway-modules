"""Pluggable feed providers for Daily News & Content Discovery Service."""

from .redis_provider import RedisFeedProvider

__all__ = ["RedisFeedProvider"]
