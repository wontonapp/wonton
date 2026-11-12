#!/bin/bash
for p in 8000 8080 8888 3000 9090 1080 18480 19480 19680 18482; do nc -z -G 2 100.83.35.101 $p && echo open $p; done
