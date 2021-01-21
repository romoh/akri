#!/bin/bash

GITHUB_TOKEN="ad93a3fb57879bf14d0b6ea497102cc60efcabc7"
EMAIL="romoh@microsoft.com"
USERNAME="romoh"

if [ -z "$GITHUB_TOKEN" ]; then
    echo "GITHUB_TOKEN is not defined"
    exit 1
fi

BRANCH_NAME="automated_cargo_update"

# assumes the repo is already cloned as a prerequisite for running the script
git checkout -b $BRANCH_NAME
cargo update && cargo test

DIFF=`git diff`
if [ -n "$DIFF" ]
then
    # configure git authorship
    git config --global user.email $EMAIL
    git config --global user.name $USERNAME

    # format: https://[USERNAME]:[TOKEN]@github.com/[USERNAME]/[REPO].git
    git remote add authenticated https://romoh:$GITHUB_TOKEN@github.com/romoh/akri.git

    # commit the changes to Cargo.lock
    git commit -a -m "Auto-update cargo crates"
    git push authenticated $BRANCH_NAME

    # finally create the PR
    curl -X POST -H "Content-Type: application/json" -H "Authorization: token $GITHUB_TOKEN" \
         --data '{"title":"Auto-update cargo crates","head":"automated_cargo_update","base":"main", "body":"Dependencies update review"}' \
         https://api.github.com/repos/romoh/akri/pulls
fi