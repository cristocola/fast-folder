# {name}

Python {python_version}+ package by {author}.

## Setup

```
# 1. Rename src/package/ to src/{name}/ first.
python -m venv .venv
source .venv/bin/activate    # Windows: .venv\Scripts\activate
pip install -e ".[dev]"
pytest
```
