import json
import os
import glob

sources = []

for meta_path in glob.glob("sources/*/source.json"):
    with open(meta_path, "r") as f:
        source = json.load(f)
    sources.append(source)

index = {
    "name": "My Aidoku Sources",          # Change this to your list name
    "website": "https://github.com/YOUR_USERNAME/aidoku-sources",
    "sources": sources
}

os.makedirs("public", exist_ok=True)

with open("public/index.json", "w") as f:
    json.dump(index, f, indent=2)

print(f"Built index with {len(sources)} source(s).")
