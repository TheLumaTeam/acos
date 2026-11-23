"""Create, start, inspect, stop, and delete an ACOS computer."""

import os

from acos import ACOSClient


with ACOSClient(token=os.environ["ACOS_TOKEN"]) as client:
    computer = client.create_computer(
        name="example-agent",
        kind="vm",
        resources={"vcpus": 2, "memory_mib": 2048},
        image="acos:latest",
    )
    computer_id = computer["id"]
    print(client.start_computer(computer_id))
    print(client.get_computer(computer_id))
    print(client.stop_computer(computer_id))
    client.delete_computer(computer_id)
