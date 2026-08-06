import requests as req

from . import helpers as hm
from .helpers import Client
from .helpers import helper as h


def run():
    h()
    hm.helper()
    Client()
    req.get("https://example.invalid")
    unknown.value()
