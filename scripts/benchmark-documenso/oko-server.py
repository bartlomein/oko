#!/usr/bin/env python3
"""Use the shared credential-isolating Oko launcher for Documenso."""
from shared import ROOT, load_engine

if __name__ == '__main__':
    load_engine('oko-server').main(root=ROOT, state_name='documenso')
