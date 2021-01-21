#!/bin/bash

TOKEN=$1
REPO="akri"
BRANCH_NAME="automated-cargo-update"
EMAIL="noreply@github.com"
USERNAME="GitHub"
ORGANIZATION=USERNAME

if [ -z "$TOKEN" ]; then
    echo "TOKEN is not defined"
    exit 1
fi

# assumes the repo is already cloned as a prerequisite for running the script
git checkout -b $BRANCH_NAME
#cargo update && cargo test

if [ -n "git diff" ]
then
    echo "Updates detected"
    # configure git authorship
    git config --global user.email $EMAIL
    git config --global user.name $USERNAME

    # format: https://[USERNAME]:[TOKEN]@github.com/[ORGANIZATION]/[REPO].git
    git remote add authenticated "https://$USERNAME:$TOKEN@github.com/$ORGANIZATION/$REPO.git"

    # commit the changes to Cargo.lock
    git commit -a -m "Auto-update cargo crates"
    
    # push the changes
    git push authenticated $BRANCH_NAME

    # finally create the PR
    curl -X POST -H "Content-Type: application/json" -H "Authorization: token $TOKEN" \
         --data '{"title":"Auto-update cargo crates","head": "'"$BRANCH_NAME"'","base":"main", "body":"Dependencies update review"}' \
         https://api.github.com/repos/$ORGANIZATION/$REPO/pulls
fi