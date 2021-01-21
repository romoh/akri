#!/bin/bash

GITHUB_TOKEN="v1nq7K+h0Uc24dgBLKSTOtifQ0OcpCqOcQHLqJuBOUo" #"8214e7ea47900b4f52546bf0865898435d8391f6"

if [ -z "$GITHUB_TOKEN" ]; then
    echo "GITHUB_TOKEN is not defined"
    exit 1
fi

BRANCH_NAME="automated_cargo_update"

git checkout -b $BRANCH_NAME
cargo update && cargo test

DIFF=`git diff`
if [ -n "$DIFF" ]; then
    # configure git authorship
    git config --global user.email $EMAIL
    git config --global user.name $USERNAME

    git remote add authenticated https://romoh:$GITHUB_TOKEN@github.com/romoh/akri.git #https://github.com/romoh/akri.git

    # commit the changes to Cargo.lock
    git commit -a -m "Auto-update cargo crates"

    # push the changes so that PR API has something to compare against
    git push authenticated $BRANCH_NAME

    # finally create the PR
    curl -X POST -H "Content-Type: application/json" -H "Authorization: token $GITHUB_TOKEN" \
         --data '{"title":"Auto-update cargo crates","head":"automated_cargo_update","base":"master", "body":"@atodorov review"}' \
         https://api.github.com/repos/romoh/akri/pulls

    # # add a remote with read/write permissions!
    # # use token authentication instead of password
    # git remote add authenticated https:/romoh/akri:$GITHUB_TOKEN@github.com/romoh/akri.git
    
    # # commit the changes to Cargo.lock
    # git commit -a -m "Auto-update cargo crates"

    # # push the changes so that PR API has something to compare against
    # git push authenticated $BRANCH_NAME

    # # finally create the PR
    # curl -X POST -H "Content-Type: application/json" -H "Authorization: token $GITHUB_TOKEN" \
    #      --data '{"title":"Auto-update cargo crates","head":"automated_cargo_update","base":"master", "body":"@atodorov review"}' \
    #      https://github.com/romoh/akri/pulls
fi