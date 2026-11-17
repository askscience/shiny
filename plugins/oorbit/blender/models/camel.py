"""A camel (desert): long legs, a hump, a long neck."""
from _quadruped import animal
from lib import blob, box


def build():
    animal(body="tint1", legs="b58e6b", length=0.5, height=0.22, girth=0.22, leg=0.34, neck=0.3,
           head=(0.12, 0.22, 0.11), snout="8f6f5a", tail="8f6f5a")
    blob("hump", "tint1", 0.15, loc=(0, 0.03, 0.56), squash=1.0, subdiv=0, seed=2)
    box("detail_blanket", "a35d5d", (0.24, 0.2, 0.03), loc=(0, 0.1, 0.55))
