"""A sheep (plains): a woolly body on thin dark legs."""
from _quadruped import animal
from lib import blob


def build():
    animal(body="eef3f2", legs="4a3b33", length=0.42, height=0.24, girth=0.26, leg=0.12, neck=0.0,
           head=(0.12, 0.16, 0.13), snout="4a3b33")
    blob("wool", "tint1", 0.24, loc=(0, 0.02, 0.26), squash=0.8, subdiv=0, seed=6, rough=0.15)
